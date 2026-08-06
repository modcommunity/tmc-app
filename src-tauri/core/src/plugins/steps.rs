//! Executes an installer plugin's plan.
//!
//! Every step is audited before and after, every path goes through the
//! [`Sandbox`], and every network fetch is bounded and host-checked. There is
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
use std::path::Path;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::audit;
use crate::error::{AppError, AppResult};
use crate::logging::Audit;
use crate::plugins::manifest::{Manifest, PathRef, Step};
use crate::plugins::sandbox::Sandbox;

/// Hard cap on a single download, when the manifest does not set a smaller one.
const DEFAULT_MAX_DOWNLOAD: u64 = 2 * 1024 * 1024 * 1024;

/// Cap on total extracted bytes, per extract step. A zip bomb is a 40 KB file
/// that becomes 5 GB; without this the sandbox's path checks are irrelevant
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
    pub sandbox: &'a Sandbox,
    pub http: &'a reqwest::Client,
    pub audit: &'a Audit,
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
                let target = self.sandbox.resolve(to, true)?;
                let resolved = ctx.fill(url);

                self.download(&resolved, &target, sha256.as_deref(), *max_bytes)
                    .await?;

                applied.push(display(&target));
            }

            Step::Extract {
                from,
                to,
                strip,
                include,
            } => {
                let source = self.sandbox.resolve(from, false)?;
                let dest = self.sandbox.resolve(to, true)?;

                let written = self.extract(&source, &dest, *strip, include)?;

                audit!(
                    self.audit,
                    Info,
                    Install,
                    "plugin.extract",
                    format!("{written} entries → {}", display(&dest)),
                    plugin = self.manifest.id
                );

                applied.push(display(&dest));
            }

            Step::Copy { from, to } => {
                let source = self.sandbox.resolve(from, false)?;
                let dest = self.sandbox.resolve(to, true)?;

                ensure_parent(&dest)?;
                std::fs::copy(&source, &dest)?;

                applied.push(display(&dest));
            }

            Step::Move { from, to } => {
                let source = self.sandbox.resolve(from, true)?;
                let dest = self.sandbox.resolve(to, true)?;

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
                let dir = self.sandbox.resolve(path, true)?;
                std::fs::create_dir_all(&dir)?;

                applied.push(display(&dir));
            }

            Step::Remove { path } => {
                let target = self.sandbox.resolve(path, true)?;

                if target.is_dir() {
                    std::fs::remove_dir_all(&target)?;
                } else if target.exists() {
                    std::fs::remove_file(&target)?;
                }

                applied.push(format!("removed {}", display(&target)));
            }

            Step::WriteText { path, content } => {
                let target = self.sandbox.resolve(path, true)?;

                ensure_parent(&target)?;
                std::fs::write(&target, ctx.fill(content))?;

                applied.push(display(&target));
            }

            Step::PatchJson {
                path,
                pointer,
                value,
            } => {
                let target = self.sandbox.resolve(path, true)?;

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
    ) -> AppResult<()> {
        let parsed = url::Url::parse(url)
            .map_err(|_| AppError::sandbox(format!("'{url}' is not a valid URL.")))?;

        // Plaintext would let anyone on the path replace a mod jar with
        // anything they like, checksum or no checksum (they would rewrite that
        // too — it comes from the same manifest, over the same channel).
        if parsed.scheme() != "https" {
            return Err(AppError::sandbox("Downloads must use https."));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::sandbox("Download URL has no host."))?;

        /*
         * Re-checked AFTER placeholder substitution, which is the whole reason
         * the check lives here rather than at manifest-validation time: a
         * template of `https://{mirror}/file.zip` is not checkable until
         * `{mirror}` is known.
         */
        if !self.manifest.allows_host(host) {
            return Err(AppError::sandbox(format!(
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
            return Err(AppError::sandbox("Download exceeds the allowed size."));
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

                return Err(AppError::sandbox("Download exceeds the allowed size."));
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

                return Err(AppError::sandbox(
                    "Downloaded file did not match the expected checksum.",
                ));
            }
        }

        Ok(())
    }

    // -------------------------------------------------------------- Extract

    fn extract(
        &self,
        source: &Path,
        dest: &Path,
        strip: u8,
        include: &[String],
    ) -> AppResult<usize> {
        let name = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        if name.ends_with(".zip") {
            self.extract_zip(source, dest, strip, include)
        } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
            let file = std::fs::File::open(source)?;
            let decoder = flate2::read::GzDecoder::new(file);

            self.extract_tar(tar::Archive::new(decoder), dest, strip, include)
        } else if name.ends_with(".tar") {
            let file = std::fs::File::open(source)?;

            self.extract_tar(tar::Archive::new(file), dest, strip, include)
        } else {
            Err(AppError::invalid(
                "Only .zip, .tar, .tar.gz and .tgz archives can be extracted.",
            ))
        }
    }

    fn extract_zip(
        &self,
        source: &Path,
        dest: &Path,
        strip: u8,
        include: &[String],
    ) -> AppResult<usize> {
        let file = std::fs::File::open(source)?;

        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| AppError::invalid(format!("Not a readable zip: {e}")))?;

        if archive.len() > MAX_EXTRACT_ENTRIES {
            return Err(AppError::sandbox("Archive has too many entries."));
        }

        let mut total: u64 = 0;
        let mut written = 0usize;

        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| AppError::invalid(format!("Bad zip entry: {e}")))?;

            /*
             * `enclosed_name` is zip-rs's own zip-slip guard: it returns None
             * for absolute paths, `..` components and Windows drive prefixes.
             * We use it AND re-check through the sandbox, because it protects
             * against the archive's own claims while the sandbox additionally
             * protects against symlinks already on disk.
             */
            let Some(raw) = entry.enclosed_name() else {
                return Err(AppError::sandbox("Archive contains an unsafe path."));
            };

            if entry.is_dir() {
                continue;
            }

            let Some(relative) = strip_and_filter(&raw, strip, include) else {
                continue;
            };

            total += entry.size();

            if total > MAX_EXTRACT_BYTES {
                return Err(AppError::sandbox(
                    "Archive expands to more than the allowed size.",
                ));
            }

            let target = crate::plugins::sandbox::join_relative(dest, &relative)?;

            if !target.starts_with(dest) {
                return Err(AppError::sandbox("Archive entry escapes its destination."));
            }

            ensure_parent(&target)?;

            let mut out = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;

            written += 1;
        }

        Ok(written)
    }

    fn extract_tar<R: Read>(
        &self,
        mut archive: tar::Archive<R>,
        dest: &Path,
        strip: u8,
        include: &[String],
    ) -> AppResult<usize> {
        let mut total: u64 = 0;
        let mut written = 0usize;

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
                return Err(AppError::sandbox(
                    "Archive contains a link or special file.",
                ));
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
                return Err(AppError::sandbox(
                    "Archive expands to more than the allowed size.",
                ));
            }

            written += 1;

            if written > MAX_EXTRACT_ENTRIES {
                return Err(AppError::sandbox("Archive has too many entries."));
            }

            let target = crate::plugins::sandbox::join_relative(dest, &relative)?;

            if !target.starts_with(dest) {
                return Err(AppError::sandbox("Archive entry escapes its destination."));
            }

            ensure_parent(&target)?;

            let mut out = std::fs::File::create(&target)?;
            std::io::copy(&mut entry, &mut out)?;
        }

        Ok(written)
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
                return Err(AppError::sandbox("Config file is too large to patch."));
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

/// Which sandbox roots a plan needs, so the UI can tell the user up front
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
}
