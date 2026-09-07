//! The deployment engine: turning a sandbox's merge tree into files the game
//! can actually see.
//!
//! Four strategies, one interface. Which one a sandbox uses is a setting, not a
//! property of the code — because no single mechanism is right for every game,
//! and the ones that are wrong are wrong in ways a user cannot debug:
//!
//! | Strategy | Game folder | Cost | Fails when |
//! | --- | --- | --- | --- |
//! | [`Strategy::Direct`] | modified | a full copy | never; it is the fallback |
//! | [`Strategy::Hardlink`] | link pointers | nothing | staging is on another drive |
//! | [`Strategy::Symlink`] | link pointers | nothing | Windows without Developer Mode |
//! | [`Strategy::Usvfs`] | untouched | nothing | not Windows, or not built with `usvfs-hooks` |
//!
//! USVFS
//! -----
//! Mod Organizer's approach: a DLL injected into the game process that hooks
//! `CreateFileW`/`GetFileAttributesW` and answers them from a merged in-memory
//! tree. It is the only strategy that leaves the game folder byte-identical,
//! and it is implemented in [`tmc_usvfs`].
//!
//! Deploying it is **not a file operation**. Nothing is placed, so:
//!
//!   * the merged tree is serialised to a blob the injected DLL maps, and that
//!     publish IS the deploy;
//!   * **the ledger comes back empty**, because a ledger records files in the
//!     game folder and there are none. That is not a gap — [`purge`] over an
//!     empty ledger correctly does nothing, and [`verify`] correctly reports a
//!     healthy folder, because the folder genuinely is untouched;
//!   * a sandbox switching TO this strategy still purges whatever the previous
//!     strategy left behind, which is the one file operation a virtual deploy
//!     performs.
//!
//! It is gated behind the `usvfs-hooks` feature and Windows. See
//! [`usvfs_unavailable`] for what each refusal means, and `tmc-usvfs`'s own
//! header for why the gate exists at all.
//!
//! FALLBACK, AND WHAT IS NOT AUTOMATIC
//! -----------------------------------
//! A hard link that cannot be made falls back to a symbolic link, and vice
//! versa, with the reason reported. Both keep the game folder made of pointers,
//! so the user's choice is honoured in the way that matters.
//!
//! Falling back to **copying** is not automatic. Copying is not a worse link,
//! it is a different decision: it writes gigabytes, it modifies the game's own
//! files, and it is the one strategy whose failure mode is a game folder that
//! needs restoring rather than unlinking. So a sandbox that asked for links and
//! can have neither gets an error naming both fixes — move the staging folder
//! to the game's drive, or choose Direct deliberately.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::plugins::jail::join_relative;

use super::ledger::{self, LedgerEntry, PurgeReport};
use super::link::{self, LinkKind, LinkSupport};
use super::merge::{self, Conflict, DeployMod, MergeTree};

/// How a sandbox's files reach the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Strategy {
    /// Copy into the real game folder, backing up whatever was there.
    ///
    /// Discouraged, and the only one that changes files the game shipped with
    /// — which is why every displaced file is moved to the backup store rather
    /// than overwritten, and why the UI asks before selecting it.
    Direct,
    /// A second directory entry for the same data. Free, invisible to the game
    /// and to every external tool, same volume only.
    Hardlink,
    /// A pointer. Crosses volumes; needs a privilege on Windows.
    Symlink,
    /// Runtime API hooking: nothing is placed and the game folder is never
    /// touched. Windows only, and behind a feature — see the module header.
    Usvfs,
}

impl Strategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Hardlink => "hardlink",
            Self::Symlink => "symlink",
            Self::Usvfs => "usvfs",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "direct" => Some(Self::Direct),
            "hardlink" | "hard" => Some(Self::Hardlink),
            "symlink" | "symbolic" => Some(Self::Symlink),
            "usvfs" | "vfs" => Some(Self::Usvfs),
            _ => None,
        }
    }

    /// Which primitive places a file, or `None` for a strategy that does not
    /// place files at all.
    pub fn link_kind(self) -> Option<LinkKind> {
        match self {
            Self::Direct => Some(LinkKind::Copy),
            Self::Hardlink => Some(LinkKind::Hard),
            Self::Symlink => Some(LinkKind::Symbolic),
            Self::Usvfs => None,
        }
    }

    /// Does the game folder end up holding real, modified files?
    pub fn modifies_game_files(self) -> bool {
        matches!(self, Self::Direct)
    }

    pub const ALL: [Strategy; 4] = [
        Strategy::Direct,
        Strategy::Hardlink,
        Strategy::Symlink,
        Strategy::Usvfs,
    ];
}

impl Default for Strategy {
    /// Hard links: free, invisible to the game, and the one that leaves the
    /// game folder recoverable by deleting pointers rather than by restoring
    /// backups. It is what Vortex defaults to, for the same reasons.
    fn default() -> Self {
        Self::Hardlink
    }
}

/// What a machine can do for one particular (staging, game) pair.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyReport {
    pub strategy: String,
    pub available: bool,
    /// Present when `available` is false: what to tell the user.
    pub reason: Option<String>,
}

/// Every strategy, and whether this machine can use it here.
///
/// The dropdown reads this. Offering a strategy that will fail is worse than
/// not offering it, because the failure arrives after the user has assigned
/// forty mods to the sandbox.
pub fn available_strategies(staging: &Path, target: &Path) -> Vec<StrategyReport> {
    let support = link::probe(staging, target);

    Strategy::ALL
        .iter()
        .map(|s| {
            let (available, reason) = match s {
                Strategy::Direct => (true, None),
                Strategy::Hardlink => (
                    support.hard,
                    (!support.hard).then(|| {
                        if support.same_volume {
                            "This filesystem does not support hard links.".to_string()
                        } else {
                            "The staging folder and the game are on different drives.".to_string()
                        }
                    }),
                ),
                Strategy::Symlink => (
                    support.symbolic,
                    (!support.symbolic).then(|| {
                        support
                            .notes
                            .iter()
                            .find(|n| n.starts_with("Symbolic links:"))
                            .cloned()
                            .unwrap_or_else(|| {
                                "This system will not let the app create symbolic links."
                                    .to_string()
                            })
                    }),
                ),
                Strategy::Usvfs => match usvfs_unavailable() {
                    Some(reason) => (false, Some(reason.to_string())),
                    None => (true, None),
                },
            };

            StrategyReport {
                strategy: s.as_str().to_string(),
                available,
                reason,
            }
        })
        .collect()
}

/// Why virtual-filesystem deployment cannot be used here, or `None` when it can.
///
/// Two different refusals, deliberately, because they are two different facts
/// about the user's situation and only one of them can ever change:
///
///   * **Not Windows.** The mechanism is import-table patching in a process
///     that has an import table of this shape. There is no macOS or Linux
///     equivalent that does not mean `DYLD_INSERT_LIBRARIES` or `LD_PRELOAD`
///     into a game — a different technique with different failure modes, not a
///     port of this one.
///   * **Not built with `usvfs-hooks`.** The build the user is running chose
///     not to ship injection. Saying "not available in this build" rather than
///     "not supported" is the honest version: another build of the same app on
///     the same machine could do it.
pub fn usvfs_unavailable() -> Option<&'static str> {
    if !cfg!(windows) {
        return Some(
            "Virtual-filesystem deployment works by hooking Windows file APIs inside the game, so \
             it is Windows-only. Use hard links, which leave the game folder just as recoverable.",
        );
    }

    if !cfg!(feature = "usvfs-hooks") {
        return Some(
            "Virtual-filesystem deployment is turned off in this build of the app. Use hard links \
             or symbolic links instead.",
        );
    }

    None
}

/// One deploy.
pub struct DeployRequest<'a> {
    pub strategy: Strategy,
    /// The game directory. Already validated as a jail anchor by the caller.
    pub target: &'a Path,
    /// Where this sandbox's mods are staged. Probed for link support and never
    /// written to here.
    pub staging: &'a Path,
    /// Where displaced originals go.
    pub backup_root: &'a Path,
    /// Where [`Strategy::Usvfs`] publishes its tree for the injected DLL to
    /// map. Required by that strategy and ignored by every other.
    pub vfs_blob: Option<&'a Path>,
    /// Every enabled mod, in any order — [`merge::build`] sorts them.
    pub mods: &'a [DeployMod],
    /// The ledger from the last deploy of this sandbox.
    pub previous: &'a [LedgerEntry],
    /// Work out the whole plan and touch nothing.
    pub dry_run: bool,
}

/// What a deploy did, or would do.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployReport {
    pub requested: String,
    /// May differ from `requested` — see `fellBack`.
    pub used: String,
    pub fell_back: Option<String>,

    pub dry_run: bool,

    /// Files newly linked or copied into the game folder.
    pub placed: usize,
    /// Files already correct from the previous deploy and left untouched. The
    /// number that makes reordering a large list cheap.
    pub reused: usize,
    /// Files removed because they are no longer wanted.
    pub removed: usize,
    /// Originals put back by that removal.
    pub restored: usize,
    /// Files displaced by this deploy and moved to the backup store.
    pub backed_up: usize,

    pub conflicts: Vec<Conflict>,
    pub empty: Vec<String>,
    pub skipped: Vec<String>,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,

    /// The new ledger. The caller persists it; the webview never sees it.
    #[serde(skip)]
    pub ledger: Vec<LedgerEntry>,
}

impl DeployReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Decide which strategy will actually be used here.
pub fn resolve_strategy(
    requested: Strategy,
    staging: &Path,
    target: &Path,
) -> AppResult<(Strategy, Option<String>, LinkSupport)> {
    if requested == Strategy::Usvfs {
        if let Some(reason) = usvfs_unavailable() {
            return Err(AppError::invalid(reason));
        }

        /*
         * No fallback, in either direction. A virtual deploy and a linked one
         * differ in whether the game folder is modified at all, which is the
         * reason somebody picks this — silently linking instead would put files
         * in a folder the user chose this strategy to keep clean, and silently
         * going virtual would leave a game that was never told about the mods.
         */
        return Ok((Strategy::Usvfs, None, link::probe(staging, target)));
    }

    let support = link::probe(staging, target);

    if requested == Strategy::Direct {
        return Ok((Strategy::Direct, None, support));
    }

    let (wanted_ok, other, other_ok) = match requested {
        Strategy::Hardlink => (support.hard, Strategy::Symlink, support.symbolic),
        _ => (support.symbolic, Strategy::Hardlink, support.hard),
    };

    if wanted_ok {
        return Ok((requested, None, support));
    }

    if other_ok {
        let why = format!(
            "{} deployment is not possible here ({}), so {} links were used instead.",
            requested.as_str(),
            first_note_for(&support, requested),
            other.as_str()
        );

        return Ok((other, Some(why), support));
    }

    /*
     * Neither link works. NOT falling through to copying: see the module
     * header. The message names both fixes because which one is right depends
     * on a decision only the user can make.
     */
    Err(AppError::invalid(format!(
        "Neither hard links nor symbolic links can be created between the app's staging folder \
         and this game folder{}. Move the sandbox's folder to the same drive as the game, or \
         switch this sandbox to Direct — which copies files into the game folder itself.",
        if support.notes.is_empty() {
            String::new()
        } else {
            format!(" ({})", support.notes.join("; "))
        }
    )))
}

fn first_note_for(support: &LinkSupport, requested: Strategy) -> String {
    let prefix = match requested {
        Strategy::Hardlink => "Hard links:",
        _ => "Symbolic links:",
    };

    support
        .notes
        .iter()
        .find(|n| n.starts_with(prefix))
        .map(|n| n.trim_start_matches(prefix).trim().to_string())
        .unwrap_or_else(|| "unsupported here".into())
}

/// Deploy a sandbox.
pub fn deploy(req: &DeployRequest<'_>) -> AppResult<DeployReport> {
    let tree = merge::build(req.mods)?;

    let (used, fell_back, _support) = resolve_strategy(req.strategy, req.staging, req.target)?;

    let mut report = DeployReport {
        requested: req.strategy.as_str().to_string(),
        used: used.as_str().to_string(),
        fell_back,
        dry_run: req.dry_run,
        conflicts: tree.conflicts.clone(),
        empty: tree.empty.clone(),
        skipped: tree.skipped.clone(),
        ..Default::default()
    };

    if used == Strategy::Usvfs {
        return deploy_virtual(req, &tree, report);
    }

    // Unreachable for anything else: every remaining strategy places files.
    let kind = used.link_kind().ok_or_else(|| {
        AppError::internal("a strategy that places no files reached the placement path")
    })?;

    // ------------------------------------------------------------- The diff
    //
    // A previous entry survives only if the tree still wants exactly it: same
    // path, same source file, same mechanism, and the file on disk is still the
    // one we put there. Anything else is purged and re-placed, which is what
    // makes a changed mod version actually replace the old file rather than
    // leaving a link to a jar that is no longer in staging.
    let keep = surviving(req, &tree, kind);

    let stale: Vec<LedgerEntry> = req
        .previous
        .iter()
        .filter(|e| !keep.contains_key(&e.path))
        .cloned()
        .collect();

    if req.dry_run {
        report.removed = stale.len();
        report.reused = keep.len();
        report.placed = tree.files.len() - keep.len();

        return Ok(report);
    }

    let purged: PurgeReport = ledger::purge(req.target, &stale);

    report.removed = purged.removed;
    report.restored = purged.restored;
    report.errors.extend(purged.errors);

    for path in purged.kept {
        report.warnings.push(format!(
            "{path} was changed since it was deployed and has been left alone."
        ));
    }

    // ------------------------------------------------------------- Placement
    for (rel, winner) in &tree.files {
        if let Some(existing) = keep.get(rel) {
            report.reused += 1;
            report.ledger.push((*existing).clone());

            continue;
        }

        let abs = match join_relative(req.target, rel) {
            Ok(abs) => abs,
            Err(e) => {
                report.errors.push(format!("{rel}: {e}"));

                continue;
            }
        };

        let mut backup = None;

        /*
         * Something is already at this path and it is not ours — the game
         * shipped it, or another manager put it there. It is MOVED to the
         * backup store, never overwritten: it may be the only copy, and a mod
         * manager that eats a base-game asset is one nobody trusts again.
         */
        if std::fs::symlink_metadata(&abs).is_ok() {
            match ledger::back_up(&abs, rel, req.backup_root) {
                Ok(at) => {
                    backup = Some(at);
                    report.backed_up += 1;
                }
                Err(e) => {
                    report.errors.push(format!(
                        "{rel}: could not set aside the existing file ({e})"
                    ));

                    continue;
                }
            }
        }

        if let Err(e) = link::place(kind, &winner.source, &abs) {
            report.errors.push(format!("{rel}: {}", e.message()));

            // Put the displaced original straight back — the placement it made
            // room for did not happen.
            if let Some(at) = &backup {
                let _ = std::fs::rename(at, &abs);
            }

            continue;
        }

        report.placed += 1;

        report.ledger.push(LedgerEntry {
            path: rel.clone(),
            kind: kind.as_str().to_string(),
            mod_key: winner.mod_key.clone(),
            source: winner.source.to_string_lossy().into_owned(),
            size: winner.size,
            mtime_ms: ledger::mtime_ms(&abs),
            backup,
        });
    }

    Ok(report)
}

/// Deploy by publishing a virtual tree instead of placing files.
///
/// The game folder is not written to at all — except to undo whatever a
/// PREVIOUS strategy put there, which is the one thing a virtual deploy has to
/// do to disk. A sandbox switched from hard links to this one otherwise keeps
/// its links, and the user gets both the links and the virtual view: every file
/// twice, with the virtual copy winning only for the paths the tree covers.
fn deploy_virtual(
    req: &DeployRequest<'_>,
    tree: &MergeTree,
    mut report: DeployReport,
) -> AppResult<DeployReport> {
    let blob = req.vfs_blob.ok_or_else(|| {
        AppError::internal("a virtual deploy was requested with nowhere to publish it")
    })?;

    let mut virtual_tree = tmc_usvfs::VirtualTree::new(req.target.to_string_lossy().into_owned());

    for (rel, winner) in &tree.files {
        let source = winner.source.to_string_lossy();

        if !virtual_tree.insert(rel, &source) {
            /*
             * The tree refuses a path it cannot represent — over its entry cap,
             * absurdly long, or a shape that could not be a relative Windows
             * path. An error rather than a warning: unlike a failed placement,
             * which leaves the other files working, a mapping the game will
             * never be told about is a mod that silently does nothing.
             */
            report
                .errors
                .push(format!("{rel}: cannot be represented in the virtual tree"));
        }
    }

    report.placed = virtual_tree.len();

    if req.dry_run {
        report.removed = req.previous.len();

        return Ok(report);
    }

    // Whatever the last strategy left in the game folder. Empty when the last
    // deploy was also virtual, which is the common case.
    let purged: PurgeReport = ledger::purge(req.target, req.previous);

    report.removed = purged.removed;
    report.restored = purged.restored;
    report.errors.extend(purged.errors);

    for path in purged.kept {
        report.warnings.push(format!(
            "{path} was changed since it was deployed and has been left alone."
        ));
    }

    /*
     * The revision is a hash of the tree, not a counter and not a clock. The
     * injected DLL uses it to notice that a blob it already read has changed,
     * and a hash makes that comparison mean "the mapping is different" — a
     * counter would also tick for a redeploy that produced an identical tree,
     * and a clock would tick on every single one.
     */
    let revision = tree_revision(&virtual_tree);

    tmc_usvfs::publish(blob, revision, &virtual_tree).map_err(|e| {
        AppError::internal(format!(
            "could not publish the virtual filesystem to {}: {e}",
            blob.display()
        ))
    })?;

    /*
     * The ledger stays EMPTY, and that is the correct record: it names files in
     * the game folder, and this deploy put none there. `purge` over it does
     * nothing because there is nothing to undo, and `verify` reports a healthy
     * folder because the folder is untouched.
     */
    Ok(report)
}

/// Remove a published virtual tree.
///
/// Undeploying a virtual sandbox is exactly this — the game folder needs no
/// work — and a missing blob is success, because the outcome asked for is "no
/// tree published" and that is already true.
pub fn purge_vfs(blob: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(blob) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Where a sandbox publishes its virtual tree.
///
/// Inside the sandbox's own staging folder, so deleting the sandbox deletes it
/// and there is no second directory to keep in step. The name is not a mod key,
/// so it cannot collide with one: [`safe_component`] never produces a leading
/// dot.
pub fn vfs_blob(root: &Path, sandbox_id: i64) -> PathBuf {
    stage_root(root, sandbox_id).join(".tmc-vfs.bin")
}

/// A content hash of the published tree.
fn tree_revision(tree: &tmc_usvfs::VirtualTree) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    for mapping in tree.mappings() {
        for byte in mapping
            .virtual_path
            .as_bytes()
            .iter()
            .chain(mapping.real_path.as_bytes())
        {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
    }

    hash
}

/// Previous entries the new tree still wants, keyed by path.
fn surviving<'a>(
    req: &DeployRequest<'a>,
    tree: &MergeTree,
    kind: LinkKind,
) -> HashMap<String, &'a LedgerEntry> {
    let mut out = HashMap::new();

    for entry in req.previous {
        let Some(winner) = tree.files.get(&entry.path) else {
            continue;
        };

        if entry.kind != kind.as_str() {
            continue;
        }

        if Path::new(&entry.source) != winner.source {
            continue;
        }

        let Ok(abs) = join_relative(req.target, &entry.path) else {
            continue;
        };

        if !ledger::is_still_ours(entry, &abs) {
            continue;
        }

        out.insert(entry.path.clone(), entry);
    }

    out
}

/// Undeploy everything a sandbox put in place.
pub fn purge(target: &Path, previous: &[LedgerEntry]) -> PurgeReport {
    ledger::purge(target, previous)
}

/// What the last deploy left behind, checked against the disk.
///
/// Run at launch. The failure this catches is the one the PDF's blueprint calls
/// orphaned links: the app was killed mid-deploy, so some files are placed and
/// the ledger does not know about them, or the ledger names files that are
/// gone. Neither is fatal and neither is visible without asking.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyReport {
    pub total: usize,
    pub intact: usize,
    /// Named by the ledger, absent from disk.
    pub missing: Vec<String>,
    /// Present, but no longer the file we deployed.
    pub changed: Vec<String>,
}

impl VerifyReport {
    pub fn healthy(&self) -> bool {
        self.missing.is_empty() && self.changed.is_empty()
    }
}

/// How many problem paths to name before summarising. A sandbox whose game
/// folder was deleted wholesale would otherwise produce a 40,000-line report.
const MAX_REPORTED: usize = 50;

pub fn verify(target: &Path, entries: &[LedgerEntry]) -> VerifyReport {
    let mut report = VerifyReport {
        total: entries.len(),
        ..Default::default()
    };

    for entry in entries {
        let Ok(abs) = join_relative(target, &entry.path) else {
            push_bounded(&mut report.changed, entry.path.clone());

            continue;
        };

        if std::fs::symlink_metadata(&abs).is_err() {
            push_bounded(&mut report.missing, entry.path.clone());
        } else if ledger::is_still_ours(entry, &abs) {
            report.intact += 1;
        } else {
            push_bounded(&mut report.changed, entry.path.clone());
        }
    }

    report
}

fn push_bounded(list: &mut Vec<String>, value: String) {
    if list.len() < MAX_REPORTED {
        list.push(value);
    } else if list.len() == MAX_REPORTED {
        list.push("…".into());
    }
}

/// Where a sandbox stages one mod's files.
///
/// `<root>/<sandbox id>/<mod key>`, with the key sanitised into a single safe
/// component. Per-mod rather than one shared folder because the merge tree's
/// whole model is "this mod provides these paths" — a shared folder cannot
/// answer which mod provided what, so it cannot report a conflict or uninstall
/// one mod.
pub fn stage_dir(root: &Path, sandbox_id: i64, mod_key: &str) -> PathBuf {
    root.join(sandbox_id.to_string())
        .join(safe_component(mod_key))
}

/// The folder holding every mod's staging directory for one sandbox.
pub fn stage_root(root: &Path, sandbox_id: i64) -> PathBuf {
    root.join(sandbox_id.to_string())
}

/// Where a sandbox's displaced originals live.
pub fn backup_root(root: &Path, sandbox_id: i64) -> PathBuf {
    root.join(sandbox_id.to_string())
}

/// A single path component that cannot be anything else.
///
/// Mod keys are `kind:id` — the colon alone would make this an alternate data
/// stream on Windows, which is a file that looks absent in every listing.
fn safe_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());

    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        } else {
            out.push('-');
        }
    }

    let trimmed = out.trim_matches(|c| c == '.' || c == '-').to_string();

    if trimmed.is_empty() || trimmed.len() > 96 {
        return format!("k{:x}", fnv1a(raw));
    }

    trimmed
}

/// A short, stable hash for a key that sanitised to nothing usable. FNV-1a
/// rather than SHA-256: this names a folder, it is not a security boundary,
/// and the shorter name keeps deep staging paths under Windows' limit.
fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }

    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _tmp: tempfile::TempDir,
        staging: PathBuf,
        game: PathBuf,
        backups: PathBuf,
        blob: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().expect("tempdir");

        let staging = tmp.path().join("staging");
        let game = tmp.path().join("game");
        let backups = tmp.path().join("backups");

        for dir in [&staging, &game, &backups] {
            std::fs::create_dir_all(dir).expect("mkdir");
        }

        Fixture {
            blob: staging.join(".tmc-vfs.bin"),
            _tmp: tmp,
            staging,
            game,
            backups,
        }
    }

    fn stage(fx: &Fixture, key: &str, rel: &str, body: &str) -> DeployMod {
        let root = fx.staging.join(key);
        let path = root.join(rel);

        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");

        DeployMod {
            key: key.into(),
            name: key.into(),
            root,
            priority: 0,
        }
    }

    fn request<'a>(
        fx: &'a Fixture,
        strategy: Strategy,
        mods: &'a [DeployMod],
        previous: &'a [LedgerEntry],
    ) -> DeployRequest<'a> {
        DeployRequest {
            strategy,
            target: &fx.game,
            staging: &fx.staging,
            backup_root: &fx.backups,
            vfs_blob: Some(&fx.blob),
            mods,
            previous,
            dry_run: false,
        }
    }

    #[test]
    fn a_direct_deploy_places_files_and_a_purge_takes_them_back_off() {
        let fx = fixture();
        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "aaa")];

        let report = deploy(&request(&fx, Strategy::Direct, &mods, &[])).expect("deploy");

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(report.placed, 1);
        assert_eq!(report.used, "direct");
        assert!(fx.game.join("mods/a.jar").exists());

        let purged = purge(&fx.game, &report.ledger);

        assert_eq!(purged.removed, 1);
        assert!(!fx.game.join("mods/a.jar").exists());
    }

    #[test]
    fn an_existing_game_file_is_set_aside_and_put_back() {
        let fx = fixture();

        std::fs::create_dir_all(fx.game.join("mods")).expect("mkdir");
        std::fs::write(fx.game.join("mods/a.jar"), b"stock").expect("write");

        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "modded")];

        let report = deploy(&request(&fx, Strategy::Direct, &mods, &[])).expect("deploy");

        assert_eq!(report.backed_up, 1);
        assert_eq!(
            std::fs::read_to_string(fx.game.join("mods/a.jar")).expect("read"),
            "modded"
        );

        let purged = purge(&fx.game, &report.ledger);

        assert_eq!(purged.restored, 1);
        assert_eq!(
            std::fs::read_to_string(fx.game.join("mods/a.jar")).expect("read"),
            "stock",
            "the game's own file must come back"
        );
    }

    /// The property that makes reordering a 500-mod list usable: a second
    /// deploy with the same inputs must touch nothing.
    #[test]
    fn redeploying_the_same_tree_reuses_everything() {
        let fx = fixture();
        let mods = vec![
            stage(&fx, "mod-a", "mods/a.jar", "aaa"),
            stage(&fx, "mod-b", "mods/b.jar", "bbb"),
        ];

        let first = deploy(&request(&fx, Strategy::Direct, &mods, &[])).expect("deploy");

        assert_eq!(first.placed, 2);

        let second = deploy(&request(&fx, Strategy::Direct, &mods, &first.ledger)).expect("deploy");

        assert_eq!(second.reused, 2);
        assert_eq!(second.placed, 0);
        assert_eq!(second.removed, 0);
        assert_eq!(second.backed_up, 0, "a reuse must not re-back-up anything");
    }

    #[test]
    fn a_removed_mod_has_its_files_taken_out_on_the_next_deploy() {
        let fx = fixture();

        let a = stage(&fx, "mod-a", "mods/a.jar", "aaa");
        let b = stage(&fx, "mod-b", "mods/b.jar", "bbb");

        let both = vec![a.clone(), b];

        let first = deploy(&request(&fx, Strategy::Direct, &both, &[])).expect("deploy");

        assert_eq!(first.placed, 2);

        let only_a = vec![a];
        let second =
            deploy(&request(&fx, Strategy::Direct, &only_a, &first.ledger)).expect("deploy");

        assert_eq!(second.removed, 1);
        assert_eq!(second.reused, 1);
        assert!(fx.game.join("mods/a.jar").exists());
        assert!(!fx.game.join("mods/b.jar").exists());
    }

    #[test]
    fn changing_priority_swaps_which_file_is_deployed() {
        let fx = fixture();

        let mut low = stage(&fx, "low", "textures/sky.dds", "low");
        let mut high = stage(&fx, "high", "textures/sky.dds", "high");

        low.priority = 1;
        high.priority = 2;

        let first = deploy(&request(
            &fx,
            Strategy::Direct,
            &[low.clone(), high.clone()],
            &[],
        ))
        .expect("deploy");

        assert_eq!(first.conflicts.len(), 1);
        assert_eq!(
            std::fs::read_to_string(fx.game.join("textures/sky.dds")).expect("read"),
            "high"
        );

        // Flip them.
        low.priority = 9;

        let second =
            deploy(&request(&fx, Strategy::Direct, &[low, high], &first.ledger)).expect("deploy");

        assert_eq!(second.placed, 1, "the winner changed, so it is re-placed");
        assert_eq!(
            std::fs::read_to_string(fx.game.join("textures/sky.dds")).expect("read"),
            "low"
        );
    }

    #[test]
    fn a_dry_run_touches_nothing() {
        let fx = fixture();
        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "aaa")];

        let mut req = request(&fx, Strategy::Direct, &mods, &[]);
        req.dry_run = true;

        let report = deploy(&req).expect("deploy");

        assert!(report.dry_run);
        assert_eq!(report.placed, 1, "the plan still counts what it would do");
        assert!(!fx.game.join("mods/a.jar").exists());
    }

    /// The picker and the deploy have to agree, whichever way the gate is set.
    /// A dropdown offering a strategy that then refuses is the failure this
    /// test exists to prevent, and it is one a feature flag makes easy.
    #[test]
    fn the_strategy_picker_agrees_with_what_a_deploy_would_do() {
        let fx = fixture();

        let listed = available_strategies(&fx.staging, &fx.game);
        let usvfs = listed
            .iter()
            .find(|s| s.strategy == "usvfs")
            .expect("listed");

        match usvfs_unavailable() {
            Some(reason) => {
                assert!(!usvfs.available);
                assert_eq!(usvfs.reason.as_deref(), Some(reason));

                let err =
                    deploy(&request(&fx, Strategy::Usvfs, &[], &[])).expect_err("must refuse");

                assert!(
                    err.to_string().contains(reason),
                    "the refusal has to say why: {err}"
                );
            }
            None => {
                assert!(usvfs.available);
                assert!(usvfs.reason.is_none());

                deploy(&request(&fx, Strategy::Usvfs, &[], &[])).expect("must not refuse");
            }
        }
    }

    /*
     * The virtual deploy is exercised through `deploy_virtual` rather than
     * through `deploy`, because `deploy` correctly refuses on a platform with
     * nothing to inject into and these tests run on all of them. What is under
     * test here is the part that is platform-independent and that the injected
     * DLL depends on being right: which mappings get published, and what
     * happens to the files the last strategy left behind.
     */
    fn virtual_deploy(fx: &Fixture, mods: &[DeployMod], previous: &[LedgerEntry]) -> DeployReport {
        let req = request(fx, Strategy::Usvfs, mods, previous);
        let tree = merge::build(req.mods).expect("merge");

        let report = DeployReport {
            requested: "usvfs".into(),
            used: "usvfs".into(),
            conflicts: tree.conflicts.clone(),
            ..Default::default()
        };

        deploy_virtual(&req, &tree, report).expect("virtual deploy")
    }

    #[test]
    fn a_virtual_deploy_publishes_a_tree_and_leaves_the_game_folder_alone() {
        let fx = fixture();
        let mods = vec![
            stage(&fx, "mod-a", "mods/a.jar", "aaa"),
            stage(&fx, "mod-b", "mods/b.jar", "bbb"),
        ];

        let report = virtual_deploy(&fx, &mods, &[]);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(report.placed, 2);

        // The whole point: nothing was written to the game.
        assert!(!fx.game.join("mods/a.jar").exists());
        assert!(!fx.game.join("mods").exists());

        // The ledger is empty, so a purge has nothing to undo and a verify
        // reports a healthy folder — both of which are true.
        assert!(report.ledger.is_empty());
        assert!(verify(&fx.game, &report.ledger).healthy());

        let published = tmc_usvfs::read(&fx.blob).expect("published");

        assert_eq!(published.tree.len(), 2);
        assert_eq!(
            published
                .tree
                .resolve("mods/a.jar")
                .map(std::path::Path::new),
            Some(fx.staging.join("mod-a/mods/a.jar")).as_deref()
        );

        purge_vfs(&fx.blob).expect("purge");
        assert!(!fx.blob.exists());
        // Undeploying twice is not an error: the outcome asked for is already
        // true the second time.
        purge_vfs(&fx.blob).expect("purge again");
    }

    /// Switching a sandbox from hard links to virtual has to take the links
    /// back off. Leaving them would give the user both — every file twice, with
    /// the virtual one winning only where the tree happens to cover it.
    #[test]
    fn switching_to_virtual_takes_the_previous_strategys_files_back_off() {
        let fx = fixture();
        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "aaa")];

        let first = deploy(&request(&fx, Strategy::Direct, &mods, &[])).expect("deploy");

        assert!(fx.game.join("mods/a.jar").exists());

        let second = virtual_deploy(&fx, &mods, &first.ledger);

        assert_eq!(second.removed, 1);
        assert!(
            !fx.game.join("mods/a.jar").exists(),
            "the linked copy is gone"
        );
        assert_eq!(second.placed, 1, "and the mapping replaced it");
    }

    /// The revision has to move when the mapping does, and stay put when it
    /// does not — it is how the injected DLL knows a blob it already read is
    /// stale.
    #[test]
    fn the_published_revision_tracks_the_tree_rather_than_the_deploy() {
        let fx = fixture();
        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "aaa")];

        virtual_deploy(&fx, &mods, &[]);
        let first = tmc_usvfs::read(&fx.blob).expect("read").revision;

        virtual_deploy(&fx, &mods, &[]);
        let again = tmc_usvfs::read(&fx.blob).expect("read").revision;

        assert_eq!(first, again, "an identical tree is not a new revision");

        let more = vec![
            stage(&fx, "mod-a", "mods/a.jar", "aaa"),
            stage(&fx, "mod-b", "mods/b.jar", "bbb"),
        ];

        virtual_deploy(&fx, &more, &[]);

        assert_ne!(
            first,
            tmc_usvfs::read(&fx.blob).expect("read").revision,
            "a changed tree is"
        );
    }

    #[test]
    fn a_hardlink_deploy_shares_storage_with_staging() {
        let fx = fixture();
        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "aaa")];

        // A filesystem with no hard links is a legitimate environment for this
        // test to run on; the fallback path is covered by its own test.
        let Ok(report) = deploy(&request(&fx, Strategy::Hardlink, &mods, &[])) else {
            return;
        };

        if report.used != "hardlink" {
            return;
        }

        assert!(link::same_file(
            &fx.game.join("mods/a.jar"),
            &fx.staging.join("mod-a/mods/a.jar")
        ));

        // And it comes back off cleanly.
        let purged = purge(&fx.game, &report.ledger);

        assert_eq!(purged.removed, 1);
        assert!(
            fx.staging.join("mod-a/mods/a.jar").exists(),
            "purging must not touch staging"
        );
    }

    #[test]
    fn verify_notices_a_file_that_went_missing() {
        let fx = fixture();
        let mods = vec![stage(&fx, "mod-a", "mods/a.jar", "aaa")];

        let report = deploy(&request(&fx, Strategy::Direct, &mods, &[])).expect("deploy");

        assert!(verify(&fx.game, &report.ledger).healthy());

        std::fs::remove_file(fx.game.join("mods/a.jar")).expect("rm");

        let checked = verify(&fx.game, &report.ledger);

        assert!(!checked.healthy());
        assert_eq!(checked.missing, vec!["mods/a.jar".to_string()]);
    }

    #[test]
    fn a_mod_key_becomes_one_safe_path_component() {
        for key in ["mod:1234", "asset:9", "../../etc", "", "a b/c"] {
            let component = safe_component(key);

            assert!(!component.contains('/'), "{key} → {component}");
            assert!(!component.contains('\\'), "{key} → {component}");
            assert!(!component.contains(':'), "{key} → {component}");
            assert!(!component.contains(".."), "{key} → {component}");
            assert!(!component.is_empty(), "{key} produced nothing");
        }

        // Stable: the same key always names the same folder.
        assert_eq!(safe_component("mod:1234"), safe_component("mod:1234"));
    }
}
