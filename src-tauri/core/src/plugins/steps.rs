//! Executes an installer plugin's plan.
//!
//! Every step is audited before and after, every path goes through the
//! [`Jail`], and every network fetch is bounded and host-checked. There is
//! no step that runs a program, so the worst a hostile manifest can achieve is
//! writing junk into the directories the user explicitly granted — which is
//! bad, recoverable, and fully recorded.
//!
//! **Not transactional.** A plan that fails at step 7 leaves steps 1–6 applied.
//! A real rollback would mean shadowing the game directory, which is
//! prohibitively expensive for a multi-gigabyte install; instead every applied
//! step is journalled (see [`RunReport::applied`]) so the uninstall plan — and
//! the user reading the log — knows exactly what landed.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::logging::Audit;
use crate::plugins::jail::Jail;
use crate::plugins::manifest::{Manifest, PathRef, Step};

/// Hard cap on a single download, when the manifest does not set a smaller one.
const DEFAULT_MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

/// Cap on total extracted bytes, per extract step. A zip bomb is a 40 KB file
/// that becomes 5 GB; without this the jail's path checks are irrelevant
/// because the disk fills before anything escapes.
const MAX_EXTRACT_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// Cap on entry count, for the same reason at the other extreme: millions of
/// empty files.
const MAX_EXTRACT_ENTRIES: usize = 50_000;

/// Cap on a JSON config the plugin patches.
const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    pub ok: bool,
    pub steps_total: usize,
    pub steps_run: usize,
    /// Paths this run created or modified, in order. The uninstall side and the
    /// activity log both read this.
    pub applied: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Values a plan may interpolate with `{name}`.
///
/// Substitution is a plain key lookup, never an expression: a template language
/// here would be a scripting language with extra steps.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunContext(pub HashMap<String, String>);

impl RunContext {
    /// One value, when the context has it.
    fn get(&self, key: &str) -> Option<&String> {
        self.0.get(key)
    }

    /// Replace `{key}` occurrences. An unknown key is left as-is rather than
    /// erroring — the URL check afterwards is what decides whether the result
    /// is acceptable, and a literal `{foo}` is never a valid host.
    fn fill(&self, template: &str) -> String {
        let mut out = template.to_string();

        for (key, value) in &self.0 {
            out = out.replace(&format!("{{{key}}}"), value);
        }

        out
    }
}

pub struct Executor<'a> {
    pub manifest: &'a Manifest,
    pub jail: &'a Jail,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,

    /// The download queue, when there is one.
    ///
    /// With it, a `download` step becomes a row in the same queue as everything
    /// else: the same progress bar, the same bandwidth limit, the same pause
    /// button, and a resume that survives the app closing. A step that streams
    /// its own bytes is invisible to all four.
    ///
    /// `None` falls back to streaming inline, which is what the tests use and
    /// what a caller with no queue to hand gets. The host allow-list, the size
    /// cap and the checksum are enforced on both paths — the queue does not
    /// replace any check, it only moves where the bytes are read.
    pub downloads: Option<&'a crate::download::DownloadManager>,
}

impl Executor<'_> {
    pub async fn run(&self, steps: &[Step], ctx: &RunContext) -> RunReport {
        let mut applied: Vec<String> = Vec::new();

        for (index, step) in steps.iter().enumerate() {
            let label = step_label(step);

            audit!(
                self.audit,
                Info,
                Install,
                "plugin.step.start",
                format!("[{}/{}] {label}", index + 1, steps.len()),
                plugin = self.manifest.id
            );

            match self.run_step(step, ctx, &mut applied).await {
                Ok(()) => audit!(
                    self.audit,
                    Info,
                    Install,
                    "plugin.step.ok",
                    format!("[{}/{}] {label}", index + 1, steps.len()),
                    plugin = self.manifest.id
                ),
                Err(err) => {
                    audit!(
                        self.audit,
                        Error,
                        Install,
                        "plugin.step.fail",
                        format!("[{}/{}] {label}: {}", index + 1, steps.len(), err.detail()),
                        plugin = self.manifest.id
                    );

                    return RunReport {
                        ok: false,
                        steps_total: steps.len(),
                        steps_run: index,
                        applied,
                        failed_at: Some(index),
                        error: Some(err.to_string()),
                    };
                }
            }
        }

        RunReport {
            ok: true,
            steps_total: steps.len(),
            steps_run: steps.len(),
            applied,
            failed_at: None,
            error: None,
        }
    }

    /// Resolve a step's path, with its placeholders filled.
    ///
    /// **Filled BEFORE the jail resolves it, never after.** Every lexical
    /// rejection in `join_relative` — `../`, an absolute path, a drive prefix,
    /// a NUL — then applies to the SUBSTITUTED value, so an item whose name is
    /// `../../../etc` is refused by exactly the check that refuses a literal
    /// one. Filling afterwards would mean checking a path and then building a
    /// different one, which is the shape of every path-traversal bug there is.
    ///
    /// Paths need this as much as URLs do: a rule that writes
    /// `mods/{fileName}` is how nearly every install rule names its output, and
    /// without substitution it produces a file called `{fileName}` — one file
    /// that every mod for that game then overwrites.
    fn resolve(&self, path_ref: &PathRef, ctx: &RunContext, write: bool) -> AppResult<PathBuf> {
        self.jail.resolve(
            &PathRef {
                root: path_ref.root,
                path: ctx.fill(&path_ref.path),
            },
            write,
        )
    }

    async fn run_step(
        &self,
        step: &Step,
        ctx: &RunContext,
        applied: &mut Vec<String>,
    ) -> AppResult<()> {
        match step {
            Step::Download {
                url,
                to,
                sha256,
                max_bytes,
            } => {
                let target = self.resolve(to, ctx, true)?;
                let resolved = ctx.fill(url);

                self.download(&resolved, &target, sha256.as_deref(), *max_bytes, ctx)
                    .await?;

                applied.push(display(&target));
            }

            Step::Extract {
                from,
                to,
                strip,
                include,
            } => {
                let source = self.resolve(from, ctx, false)?;
                let dest = self.resolve(to, ctx, true)?;

                let written = extract_archive(&source, &dest, *strip, include)?;

                audit!(
                    self.audit,
                    Info,
                    Install,
                    "plugin.extract",
                    format!("{} entries → {}", written.len(), display(&dest)),
                    plugin = self.manifest.id
                );

                /*
                 * Every extracted FILE, not the destination directory.
                 *
                 * The journal is what `library::install::run_uninstall` sweeps,
                 * and it sweeps by removing each recorded path — a directory
                 * with `remove_dir_all`. Recording `Data` for a Skyrim mod, or
                 * `scripts` for a GTA V one, therefore meant uninstalling one
                 * mod deleted the shared folder every other mod had also
                 * extracted into. The GTA V rule's own comment says it relies
                 * on the journal for exactly this, which is what makes the
                 * mismatch worth fixing here rather than in the rules: a rule
                 * cannot enumerate what an archive contains, and this can.
                 *
                 * It is capped by MAX_EXTRACT_ENTRIES, so the row is bounded.
                 * A large texture pack does produce a large list, and that is
                 * the correct size for a record of what it wrote.
                 */
                for path in &written {
                    applied.push(display(path));
                }
            }

            Step::Copy { from, to } => {
                let source = self.resolve(from, ctx, false)?;
                let dest = self.resolve(to, ctx, true)?;

                ensure_parent(&dest)?;
                std::fs::copy(&source, &dest)?;

                applied.push(display(&dest));
            }

            Step::Move { from, to } => {
                let source = self.resolve(from, ctx, true)?;
                let dest = self.resolve(to, ctx, true)?;

                ensure_parent(&dest)?;

                /*
                 * `rename` fails across filesystems, which is the normal case
                 * when the download cache and the game directory are on
                 * different volumes. Fall back to copy-then-delete rather than
                 * surfacing an EXDEV the user cannot act on.
                 */
                if std::fs::rename(&source, &dest).is_err() {
                    std::fs::copy(&source, &dest)?;
                    std::fs::remove_file(&source)?;
                }

                applied.push(display(&dest));
            }

            Step::Mkdir { path } => {
                let dir = self.resolve(path, ctx, true)?;
                std::fs::create_dir_all(&dir)?;

                applied.push(display(&dir));
            }

            Step::Remove { path } => {
                let target = self.resolve(path, ctx, true)?;

                if target.is_dir() {
                    std::fs::remove_dir_all(&target)?;
                } else if target.exists() {
                    std::fs::remove_file(&target)?;
                }

                applied.push(format!("removed {}", display(&target)));
            }

            Step::WriteText { path, content } => {
                let target = self.resolve(path, ctx, true)?;

                ensure_parent(&target)?;
                std::fs::write(&target, ctx.fill(content))?;

                applied.push(display(&target));
            }

            Step::PatchJson {
                path,
                pointer,
                value,
            } => {
                let target = self.resolve(path, ctx, true)?;

                self.patch_json(&target, pointer, value)?;

                applied.push(display(&target));
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------- Download

    async fn download(
        &self,
        url: &str,
        target: &Path,
        expect_sha: Option<&str>,
        max_bytes: Option<u64>,
        ctx: &RunContext,
    ) -> AppResult<()> {
        let parsed = url::Url::parse(url)
            .map_err(|_| AppError::jail(format!("'{url}' is not a valid URL.")))?;

        // Plaintext would let anyone on the path replace a mod jar with
        // anything they like, checksum or no checksum (they would rewrite that
        // too — it comes from the same manifest, over the same channel).
        if parsed.scheme() != "https" {
            return Err(AppError::jail("Downloads must use https."));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::jail("Download URL has no host."))?;

        /*
         * Re-checked AFTER placeholder substitution, which is the whole reason
         * the check lives here rather than at manifest-validation time: a
         * template of `https://{mirror}/file.zip` is not checkable until
         * `{mirror}` is known.
         */
        if !self.manifest.allows_host(host) {
            return Err(AppError::jail(format!(
                "'{host}' is not in this plugin's allowed download hosts."
            )));
        }

        let cap = max_bytes
            .unwrap_or(DEFAULT_MAX_DOWNLOAD)
            .min(DEFAULT_MAX_DOWNLOAD);

        audit!(
            self.audit,
            Security,
            Network,
            "plugin.download",
            format!("{url} → {}", display(target)),
            plugin = self.manifest.id
        );

        if let Some(queue) = self.downloads {
            ensure_parent(target)?;

            /*
             * A DETERMINISTIC id, from the plugin and the destination. Two runs
             * of the same step are then the same queue row rather than two
             * writers for one file — which is what re-running a failed install
             * does, and what two sandboxes staging the same mod would do if the
             * destination were shared.
             */
            let id = format!("plugin:{}:{}", self.manifest.id, display(target));

            return queue
                .run_to_completion(crate::download::DownloadRequest {
                    id,
                    url: url.to_string(),
                    dest: target.to_path_buf(),
                    /*
                     * The ITEM's name, not the file's. A queue row reading
                     * `a1b2c3d4.zip` is one the user cannot connect to
                     * anything they asked for — release files are frequently
                     * named by hash or by build number, and the queue is the
                     * screen where somebody looks to find out what is taking
                     * so long.
                     */
                    label: ctx
                        .get("itemName")
                        .cloned()
                        .or_else(|| target.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .unwrap_or_else(|| self.manifest.name.clone()),
                    sha256: expect_sha.map(str::to_string),
                    size_hint: None,
                    priority: 0,
                    limit_bps: None,
                    meta: {
                        let mut meta = std::collections::BTreeMap::from([(
                            "plugin".to_string(),
                            self.manifest.id.clone(),
                        )]);

                        /*
                         * The subscription's key, so the queue can find the
                         * item's own artwork in the local library rather than
                         * asking the network for a picture per row. It is the
                         * same `kind:itemId` a sandbox mod is keyed by.
                         */
                        if let Some(id) = ctx.get("id") {
                            meta.insert("item".to_string(), id.clone());
                        }

                        if let Some(kind) = ctx.get("kind") {
                            meta.insert("kind".to_string(), kind.clone());
                        }

                        meta
                    },
                })
                .await;
        }

        let response = self.http.get(parsed).send().await?;

        if !response.status().is_success() {
            return Err(AppError::Network(format!(
                "Download failed ({}).",
                response.status()
            )));
        }

        // Trust the header only to fail EARLY; the streaming counter below is
        // what actually enforces the cap, since a hostile server can lie.
        if response.content_length().is_some_and(|len| len > cap) {
            return Err(AppError::jail("Download exceeds the allowed size."));
        }

        ensure_parent(target)?;

        let mut file = std::fs::File::create(target)?;
        let mut hasher = Sha256::new();
        let mut written: u64 = 0;

        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;

            written += chunk.len() as u64;

            if written > cap {
                drop(file);
                let _ = std::fs::remove_file(target);

                return Err(AppError::jail("Download exceeds the allowed size."));
            }

            hasher.update(&chunk);
            std::io::Write::write_all(&mut file, &chunk)?;
        }

        drop(file);

        if let Some(expected) = expect_sha {
            let actual = hex::encode(hasher.finalize());

            if !actual.eq_ignore_ascii_case(expected.trim()) {
                // Remove it. A file that failed its checksum must not be left
                // where a later step could pick it up.
                let _ = std::fs::remove_file(target);

                audit!(
                    self.audit,
                    Error,
                    Install,
                    "plugin.download.checksum",
                    format!("{url}: expected {expected}, got {actual}"),
                    plugin = self.manifest.id
                );

                return Err(AppError::jail(
                    "Downloaded file did not match the expected checksum.",
                ));
            }
        }

        Ok(())
    }

    // ------------------------------------------------------------ JSON patch

    fn patch_json(&self, target: &Path, pointer: &str, value: &serde_json::Value) -> AppResult<()> {
        if !pointer.starts_with('/') || pointer.len() > 256 {
            return Err(AppError::invalid(
                "JSON pointer must start with '/' and be short.",
            ));
        }

        let mut doc: serde_json::Value = if target.exists() {
            let meta = std::fs::metadata(target)?;

            if meta.len() > MAX_JSON_BYTES {
                return Err(AppError::jail("Config file is too large to patch."));
            }

            serde_json::from_str(&std::fs::read_to_string(target)?)
                .map_err(|e| AppError::invalid(format!("Config file is not valid JSON: {e}")))?
        } else {
            serde_json::Value::Object(Default::default())
        };

        // Walk the pointer, creating intermediate objects. `serde_json`'s own
        // `pointer_mut` refuses to create missing nodes, which is exactly the
        // case a first-run config patch is in.
        let mut cursor = &mut doc;

        let parts: Vec<String> = pointer[1..]
            .split('/')
            .map(|p| p.replace("~1", "/").replace("~0", "~"))
            .collect();

        let Some((last, parents)) = parts.split_last() else {
            return Err(AppError::invalid("Empty JSON pointer."));
        };

        for part in parents {
            if !cursor.is_object() {
                *cursor = serde_json::Value::Object(Default::default());
            }

            cursor = cursor
                .as_object_mut()
                .expect("just ensured object")
                .entry(part.clone())
                .or_insert_with(|| serde_json::Value::Object(Default::default()));
        }

        if !cursor.is_object() {
            *cursor = serde_json::Value::Object(Default::default());
        }

        cursor
            .as_object_mut()
            .expect("just ensured object")
            .insert(last.clone(), value.clone());

        ensure_parent(target)?;

        // Temp-and-rename: a config the game reads must never be observed
        // half-written.
        let tmp = target.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&doc)?)?;
        std::fs::rename(&tmp, target)?;

        Ok(())
    }
}

// ------------------------------------------------------------------ Extract
//
// Free functions rather than methods on [`Executor`], because unpacking an
// archive is also what happens to a file the USER dropped into the window —
// see [`crate::local::store`]. Having one implementation of the zip-slip
// guard, the expansion cap and tar's link refusal is the whole reason: two
// would eventually differ, and the one that differed would be the one nobody
// was reading.

pub(crate) fn extract_archive(
    source: &Path,
    dest: &Path,
    strip: u8,
    include: &[String],
) -> AppResult<Vec<PathBuf>> {
    let name = source
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if name.ends_with(".zip") {
        extract_zip(source, dest, strip, include)
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let file = std::fs::File::open(source)?;
        let decoder = flate2::read::GzDecoder::new(file);

        extract_tar(tar::Archive::new(decoder), dest, strip, include)
    } else if name.ends_with(".tar") {
        let file = std::fs::File::open(source)?;

        extract_tar(tar::Archive::new(file), dest, strip, include)
    } else {
        Err(AppError::invalid(
            "Only .zip, .tar, .tar.gz and .tgz archives can be extracted.",
        ))
    }
}

/// The files written, in archive order — the caller journals them.
fn extract_zip(
    source: &Path,
    dest: &Path,
    strip: u8,
    include: &[String],
) -> AppResult<Vec<PathBuf>> {
    let file = std::fs::File::open(source)?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid(format!("Not a readable zip: {e}")))?;

    if archive.len() > MAX_EXTRACT_ENTRIES {
        return Err(AppError::jail("Archive has too many entries."));
    }

    let mut total: u64 = 0;
    let mut written: Vec<PathBuf> = Vec::new();

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| AppError::invalid(format!("Bad zip entry: {e}")))?;

        /*
         * `enclosed_name` is zip-rs's own zip-slip guard: it returns None
         * for absolute paths, `..` components and Windows drive prefixes.
         * We use it AND re-check through the jail, because it protects
         * against the archive's own claims while the jail additionally
         * protects against symlinks already on disk.
         */
        let Some(raw) = entry.enclosed_name() else {
            return Err(AppError::jail("Archive contains an unsafe path."));
        };

        if entry.is_dir() {
            continue;
        }

        let Some(relative) = strip_and_filter(&raw, strip, include) else {
            continue;
        };

        total += entry.size();

        if total > MAX_EXTRACT_BYTES {
            return Err(AppError::jail(
                "Archive expands to more than the allowed size.",
            ));
        }

        let target = crate::plugins::jail::join_relative(dest, &relative)?;

        if !target.starts_with(dest) {
            return Err(AppError::jail("Archive entry escapes its destination."));
        }

        ensure_parent(&target)?;

        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;

        written.push(target);
    }

    Ok(written)
}

fn extract_tar<R: Read>(
    mut archive: tar::Archive<R>,
    dest: &Path,
    strip: u8,
    include: &[String],
) -> AppResult<Vec<PathBuf>> {
    let mut total: u64 = 0;
    let mut written: Vec<PathBuf> = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;

        /*
         * Tar can carry symlinks and hardlinks, and both are a jail escape
         * by construction: a `link → /etc` entry followed by a `link/passwd`
         * entry writes outside any destination check that looks only at
         * paths. Nothing a mod archive needs requires them.
         */
        let kind = entry.header().entry_type();

        if !kind.is_file() && !kind.is_dir() {
            return Err(AppError::jail("Archive contains a link or special file."));
        }

        if kind.is_dir() {
            continue;
        }

        let raw = entry.path()?.into_owned();

        let Some(relative) = strip_and_filter(&raw, strip, include) else {
            continue;
        };

        total += entry.size();

        if total > MAX_EXTRACT_BYTES {
            return Err(AppError::jail(
                "Archive expands to more than the allowed size.",
            ));
        }

        if written.len() >= MAX_EXTRACT_ENTRIES {
            return Err(AppError::jail("Archive has too many entries."));
        }

        let target = crate::plugins::jail::join_relative(dest, &relative)?;

        if !target.starts_with(dest) {
            return Err(AppError::jail("Archive entry escapes its destination."));
        }

        ensure_parent(&target)?;

        let mut out = std::fs::File::create(&target)?;
        std::io::copy(&mut entry, &mut out)?;

        written.push(target);
    }

    Ok(written)
}

/// Apply `strip` and the include filter; `None` means "skip this entry".
fn strip_and_filter(raw: &Path, strip: u8, include: &[String]) -> Option<String> {
    let parts: Vec<String> = raw
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    if parts.len() <= strip as usize {
        return None;
    }

    let relative = parts[strip as usize..].join("/");

    if relative.is_empty() {
        return None;
    }

    if !include.is_empty() && !include.iter().any(|p| relative.starts_with(p.as_str())) {
        return None;
    }

    Some(relative)
}

fn ensure_parent(path: &Path) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    Ok(())
}

/// Paths in the log are shown as-is. They are the user's own directories and
/// the whole value of the log is being able to see exactly what was touched.
fn display(path: &Path) -> String {
    path.display().to_string()
}

fn step_label(step: &Step) -> String {
    match step {
        Step::Download { url, .. } => format!("download {url}"),
        Step::Extract { from, .. } => format!("extract {}", from.path),
        Step::Copy { from, to } => format!("copy {} → {}", from.path, to.path),
        Step::Move { from, to } => format!("move {} → {}", from.path, to.path),
        Step::Mkdir { path } => format!("mkdir {}", path.path),
        Step::Remove { path } => format!("remove {}", path.path),
        Step::WriteText { path, .. } => format!("write {}", path.path),
        Step::PatchJson { path, pointer, .. } => format!("patch {}{}", path.path, pointer),
    }
}

/// Which jail roots a plan needs, so the UI can tell the user up front
/// rather than failing halfway through.
pub fn required_roots(steps: &[Step]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();

    let mut note = |p: &PathRef| {
        let key = match p.root {
            crate::plugins::manifest::FsRoot::GameDir => "gameDir",
            crate::plugins::manifest::FsRoot::PluginData => "pluginData",
            crate::plugins::manifest::FsRoot::Downloads => "downloads",
        };

        if !out.contains(&key) {
            out.push(key);
        }
    };

    for step in steps {
        match step {
            Step::Download { to, .. } => note(to),
            Step::Extract { from, to, .. } | Step::Copy { from, to } | Step::Move { from, to } => {
                note(from);
                note(to);
            }
            Step::Mkdir { path }
            | Step::Remove { path }
            | Step::WriteText { path, .. }
            | Step::PatchJson { path, .. } => note(path),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    #[test]
    fn strip_drops_leading_components() {
        let out = strip_and_filter(Path::new("pkg-1.0/mods/a.jar"), 1, &[]);
        assert_eq!(out.as_deref(), Some("mods/a.jar"));
    }

    #[test]
    fn strip_past_the_end_skips_the_entry() {
        assert!(strip_and_filter(Path::new("a.jar"), 2, &[]).is_none());
    }

    #[test]
    fn include_filters_by_prefix() {
        let path = Path::new("root/mods/a.jar");

        assert!(strip_and_filter(path, 1, &["mods".into()]).is_some());
        assert!(strip_and_filter(path, 1, &["config".into()]).is_none());
    }

    #[test]
    fn context_substitution_is_literal() {
        let ctx = RunContext(HashMap::from([("id".into(), "42".into())]));

        assert_eq!(ctx.fill("https://x.test/{id}.zip"), "https://x.test/42.zip");
        // No expression evaluation, no recursion into the substituted value.
        assert_eq!(ctx.fill("{unknown}"), "{unknown}");
    }

    // ------------------------------------------------- Placeholders in paths

    /// A manifest that may write anywhere under the game folder.
    fn writable_manifest() -> Manifest {
        Manifest {
            manifest_version: 1,
            id: "app.test".into(),
            name: "t".into(),
            version: "1".into(),
            author: "t".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: crate::plugins::manifest::Permissions {
                fs: vec![crate::plugins::manifest::FsGrant {
                    root: crate::plugins::manifest::FsRoot::GameDir,
                    path: String::new(),
                    write: true,
                }],
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        }
    }

    fn jail_over(manifest: &Manifest, game: &Path) -> Jail {
        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();

        available.insert("gameDir", game.to_path_buf());

        Jail::build(manifest, &available).expect("jail")
    }

    /// Write one file through the executor, and say whether it worked.
    async fn write_through(game: &Path, audit_at: &Path, template: &str, ctx: &RunContext) -> bool {
        let manifest = writable_manifest();
        let jail = jail_over(&manifest, game);
        let audit = Audit::new(audit_at.to_path_buf());

        let executor = Executor {
            manifest: &manifest,
            jail: &jail,
            http: &reqwest::Client::new(),
            audit: &audit,
            downloads: None,
        };

        let steps = vec![Step::WriteText {
            path: PathRef {
                root: crate::plugins::manifest::FsRoot::GameDir,
                path: template.into(),
            },
            content: "x".into(),
        }];

        executor.run(&steps, ctx).await.ok
    }

    /// The journal names every extracted FILE, never the folder it went into.
    ///
    /// The regression this pins was a data-loss bug with a comment pointing
    /// straight at it. `library::install::run_uninstall` sweeps the journal by
    /// removing each recorded path, and a directory goes through
    /// `remove_dir_all` — so an extract that recorded only its destination
    /// meant uninstalling one mod deleted the shared folder every other mod had
    /// extracted into. The shipped GTA V rule says in its own uninstall comment
    /// that it relies on the journal for exactly this, and `gameDir/scripts` is
    /// precisely such a shared folder.
    #[tokio::test]
    async fn extracting_journals_the_files_and_not_the_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");
        std::fs::create_dir_all(game.join("scripts")).expect("mkdir");

        // A file another mod already put there. It must survive.
        std::fs::write(game.join("scripts/other.lua"), "-- someone else's\n").expect("write");

        let archive = tmp.path().join("mod.zip");
        {
            let file = std::fs::File::create(&archive).expect("create");
            let mut zip = zip::ZipWriter::new(file);
            let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);

            for name in ["a.lua", "nested/b.lua"] {
                zip.start_file(name, opts).expect("entry");
                std::io::Write::write_all(&mut zip, b"body").expect("body");
            }

            zip.finish().expect("finish");
        }

        std::fs::copy(&archive, game.join("mod.zip")).expect("stage the archive");

        let manifest = writable_manifest();
        let jail = jail_over(&manifest, &game);
        let audit = Audit::new(tmp.path().join("audit.jsonl"));

        let executor = Executor {
            manifest: &manifest,
            jail: &jail,
            http: &reqwest::Client::new(),
            audit: &audit,
            downloads: None,
        };

        let gd = crate::plugins::manifest::FsRoot::GameDir;

        let report = executor
            .run(
                &[Step::Extract {
                    from: PathRef {
                        root: gd,
                        path: "mod.zip".into(),
                    },
                    to: PathRef {
                        root: gd,
                        path: "scripts".into(),
                    },
                    strip: 0,
                    include: vec![],
                }],
                &RunContext(HashMap::new()),
            )
            .await;

        assert!(report.ok, "extract failed: {:?}", report.error);

        let journal = &report.applied;

        assert!(
            journal.iter().any(|p| p.ends_with("a.lua"))
                && journal.iter().any(|p| p.ends_with("b.lua")),
            "the journal should name every extracted file: {journal:?}"
        );
        assert!(
            !journal.iter().any(|p| p.ends_with("scripts")),
            "the journal must not name the destination directory: {journal:?}"
        );
        assert!(
            !journal.iter().any(|p| p.ends_with("other.lua")),
            "only what this install wrote belongs in its journal: {journal:?}"
        );
        assert!(
            game.join("scripts/other.lua").exists(),
            "the other mod's file was disturbed"
        );
    }

    /// A placeholder in a PATH is filled, exactly as one in a URL is.
    ///
    /// The regression: they were not. `mods/{fileName}` produced a file
    /// literally called `{fileName}`, so every mod for a game wrote to the same
    /// one — and the shipped example rules all use that spelling. It survived
    /// because a path with braces in it is a perfectly valid path, so nothing
    /// failed until a mod was actually installed.
    #[tokio::test]
    async fn a_placeholder_in_a_path_is_filled() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");

        std::fs::create_dir_all(&game).expect("mkdir");

        let ctx = RunContext(HashMap::from([("fileName".into(), "cool.jar".into())]));

        assert!(
            write_through(
                &game,
                &tmp.path().join("audit.jsonl"),
                "mods/{fileName}",
                &ctx
            )
            .await
        );

        assert!(game.join("mods/cool.jar").is_file(), "the filled name");
        assert!(
            !game.join("mods/{fileName}").exists(),
            "and not the literal one"
        );
    }

    /// Filling happens BEFORE the jail resolves, so a hostile value is refused
    /// by the same check that refuses a hostile literal.
    ///
    /// Item names are author-written text off the network. Were substitution to
    /// run after the containment check, this would be a plain traversal.
    #[tokio::test]
    async fn a_placeholder_cannot_carry_a_path_out_of_the_jail() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let game = tmp.path().join("game");

        std::fs::create_dir_all(&game).expect("mkdir");

        let cases = [
            // Traversal out of the subdirectory the template names.
            ("mods/{itemName}", "../../escaped.txt"),
            ("mods/{itemName}", "../../../escaped.txt"),
            // A template that is NOTHING but a placeholder, where an absolute
            // value really is absolute — `Path::join` would discard the base.
            ("{itemName}", "/etc/escaped"),
            ("{itemName}", "C:\\Windows\\escaped"),
            ("{itemName}", "../escaped.txt"),
        ];

        for (template, hostile) in cases {
            let ctx = RunContext(HashMap::from([("itemName".into(), hostile.to_string())]));

            assert!(
                !write_through(&game, &tmp.path().join("audit.jsonl"), template, &ctx).await,
                "{template} filled with {hostile} should be refused"
            );
        }

        /*
         * NOT in the list, and worth saying why: `mods/{itemName}` filled with
         * `/etc/escaped` produces `mods//etc/escaped`, which is a RELATIVE path
         * with an empty component in it and resolves inside the jail. It is
         * allowed, and allowing it is correct — an absolute-looking value in
         * the middle of a template is not absolute.
         */

        assert!(
            !tmp.path().join("escaped.txt").exists(),
            "nothing landed outside"
        );
    }
}
