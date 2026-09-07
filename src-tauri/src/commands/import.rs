//! Bringing mods in from outside the account: dropped files, folders the game
//! already has, and other mod managers' libraries.
//!
//! THE RULE FROM `commands/mod.rs`, KEPT
//! ------------------------------------
//! **No command here takes a filesystem path from the webview.** Every path
//! this module acts on was produced by Rust — the OS delivered a drag and drop
//! event to the window, or one of the two scans below listed a directory — and
//! is handed to the frontend as an opaque token. The webview says *which of the
//! things you found*, never *this path*.
//!
//! [`tmc_core::local::vault`] holds that indirection and its header says
//! exactly what it buys and what it does not. The short version: a script that
//! has achieved execution in the webview can replay a token for a file the user
//! just dropped, which is nothing it could not achieve by asking them to drop
//! it again; it cannot name a path nobody pointed at, because no token exists
//! for one.
//!
//! So this is not the widening `commands/fs.rs` is. `fs_list_dirs` takes a path
//! and reads a directory; nothing here takes one at all.
//!
//! WHAT AN IMPORT ACTUALLY DOES
//! ----------------------------
//! Copies files into the app's own store and writes a row. It does not write to
//! a game folder — deploying a sandbox does that, later, on a separate click,
//! through the same engine every subscribed mod goes through. And it never
//! removes, moves or modifies the source: somebody trying this app out has to
//! be able to go back to Vortex the next morning and find nothing changed.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;

use tmc_core::audit;
use tmc_core::error::{AppError, AppResult};
use tmc_core::local::store::{self, ImportOptions, Payload};
use tmc_core::local::vault::is_importable;
use tmc_core::local::{adopt, LocalMod, LocalPatch, NewLocalMod, Origin, SourceRef};
use tmc_core::plugins::managers::{self, ManagerSpec};

use crate::state::AppState;

/// One thing waiting to be imported, as the webview sees it.
///
/// No path, by construction — [`Self::token`] is how it is named back. `name`
/// is the file or folder name, which is what the user recognises and what they
/// will edit afterwards if they want something better.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingFile {
    pub token: String,
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
    /// Whether this looks like an archive this app can unpack.
    pub is_archive: bool,
}

/// A batch of files delivered by one drop.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DropBatch {
    /// Milliseconds since the epoch, so the UI can ignore a batch it has
    /// already handled after a remount.
    pub at: i64,
    pub files: Vec<PendingFile>,
}

/// What importing one token would produce, before anything is written.
///
/// A preview rather than a commit, because the two decisions an import makes —
/// unpack or not, and where the payload belongs — are guesses about somebody
/// else's archive. Showing them first is the difference between a manager that
/// puts files somewhere and one you can trust with a game folder.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub token: String,
    pub name: String,
    pub is_dir: bool,
    pub bytes: u64,
    /// `file`, `unpack` or `folder`.
    pub payload: Payload,
    /// Where its contents would land under the game folder.
    pub rel_path: String,
    /// Set when the archive carried a `tmc.json` naming an item on THIS site.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}

/// A per-file override the user set in the import dialog.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOverride {
    pub token: String,
    pub name: Option<String>,
    pub rel_path: Option<String>,
    pub payload: Option<Payload>,
}

/// What one import run did.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: Vec<LocalMod>,
    /// One line per thing that did not import, naming it. A batch where three
    /// of twelve failed has to say which three — a count is not actionable.
    pub failed: Vec<String>,
    /// Sandboxes the new mods were added to, so the UI can offer a deploy.
    pub sandbox_id: Option<i64>,
}

// ------------------------------------------------------------------- Dropping

/// The files from the most recent drop on the window.
///
/// A command as well as an event, because the event fires whether or not
/// anything is listening — a drop during a route change would otherwise be
/// silently lost, which reads as drag and drop not working rather than as a
/// race.
#[tauri::command]
pub fn import_dropped(state: State<'_, AppState>) -> DropBatch {
    state.take_drop().unwrap_or_default()
}

// ------------------------------------------------------------------ Previewing

/// What importing these would do, without doing any of it.
#[tauri::command]
pub fn import_preview(
    state: State<'_, AppState>,
    tokens: Vec<String>,
    app_id: Option<i64>,
) -> AppResult<Vec<ImportPreview>> {
    let plugins = state.app_plugins();

    let slug = app_id.and_then(|id| state.app_slug_for(id));

    let targets = slug
        .as_deref()
        .map(|s| plugins.mod_targets(s))
        .unwrap_or_default();

    let extensions = slug
        .as_deref()
        .map(|s| plugins.mod_extensions(s))
        .unwrap_or_default();

    let (paths, missing) = state.vault.resolve_all(&tokens);

    if !missing.is_empty() {
        return Err(AppError::invalid(
            "Some of those files are no longer available — drop them again.",
        ));
    }

    let mut out = Vec::new();

    for (token, path) in tokens.iter().zip(paths) {
        if !is_importable(&path) {
            continue;
        }

        let is_dir = path.is_dir();
        let payload = store::suggest_payload(&path, &extensions);

        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Imported mod")
            .to_string();

        let bytes = if is_dir {
            0
        } else {
            std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
        };

        /*
         * The destination is guessed WITHOUT unpacking, which means the "is
         * this archive already game-relative?" test cannot run yet — that one
         * needs the payload's top level. So the preview shows the game's own
         * mod target and the real decision happens in `prepare`, which then
         * reports what it chose. Unpacking a batch of twelve archives to
         * populate a dialog somebody may cancel is not a trade worth making.
         */
        let rel_path = targets.first().cloned().unwrap_or_default();

        out.push(ImportPreview {
            token: token.clone(),
            name,
            is_dir,
            bytes,
            payload,
            rel_path,
            source: None,
            version: None,
            author: None,
        });
    }

    Ok(out)
}

// ------------------------------------------------------------------ Importing

/// Import everything named, into the device's store.
///
/// `sandbox_id` is optional and adds each import to that sandbox as well, which
/// is the path the drop overlay takes — dropping a jar onto a sandbox should
/// put it in that sandbox, not into a list somewhere the user then has to find.
/// Nothing is deployed either way.
#[tauri::command]
pub fn import_paths(
    state: State<'_, AppState>,
    tokens: Vec<String>,
    app_id: Option<i64>,
    sandbox_id: Option<i64>,
    overrides: Option<Vec<ImportOverride>>,
    origin: Option<String>,
) -> AppResult<ImportReport> {
    let sandbox = match sandbox_id {
        Some(id) => Some(
            state
                .library
                .sandbox_get(id)?
                .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?,
        ),
        None => None,
    };

    // The sandbox's game wins over the argument: dropping onto a Valheim
    // sandbox cannot mean "import this for Minecraft", and letting the two
    // disagree is how a mod ends up in the wrong game's folder.
    let app_id = sandbox.as_ref().map(|s| s.app_id).or(app_id);

    let slug = sandbox
        .as_ref()
        .and_then(|s| s.app_slug.clone())
        .or_else(|| app_id.and_then(|id| state.app_slug_for(id)));

    let plugins = state.app_plugins();

    let targets = slug
        .as_deref()
        .map(|s| plugins.mod_targets(s))
        .unwrap_or_default();

    let extensions = slug
        .as_deref()
        .map(|s| plugins.mod_extensions(s))
        .unwrap_or_default();

    let store_root = state.paths.local_mods_dir();
    let overrides = overrides.unwrap_or_default();

    let origin = match origin.as_deref() {
        Some("adopted") => Origin::Adopted,
        Some("manager") => Origin::Manager,
        _ => Origin::Dropped,
    };

    let mut report = ImportReport {
        sandbox_id,
        ..Default::default()
    };

    for token in &tokens {
        let Some(path) = state.vault.resolve(token) else {
            report
                .failed
                .push("One of those files is no longer available.".into());

            continue;
        };

        if !is_importable(&path) {
            report
                .failed
                .push(format!("{} is not there any more.", display_name(&path)));

            continue;
        }

        let over = overrides.iter().find(|o| &o.token == token);

        // A dropped DIRECTORY records itself as one. The distinction is only
        // ever shown to the user — "Dropped in" against "Folder" — and it is
        // the first thing they read on a row they do not recognise.
        let origin = if origin == Origin::Dropped && path.is_dir() {
            Origin::Folder
        } else {
            origin
        };

        // A folder is always a folder, whatever the dialog said — the payload
        // choice only means anything for a file.
        let payload = if path.is_dir() {
            Some(Payload::Folder)
        } else {
            over.and_then(|o| o.payload)
        };

        let opts = ImportOptions {
            mod_targets: &targets,
            mod_extensions: &extensions,
            payload,
            rel_path: over.and_then(|o| o.rel_path.clone()),
        };

        match import_one(
            &state,
            &store_root,
            &path,
            &opts,
            app_id,
            slug.as_deref(),
            origin,
            over,
        ) {
            Ok(row) => {
                if let Some(sandbox) = &sandbox {
                    if let Err(e) = state.library.sandbox_add_local(sandbox.id, &row) {
                        report.failed.push(format!("{}: {}", row.name, e));
                    }
                }

                report.imported.push(row);
            }
            Err(e) => report
                .failed
                .push(format!("{}: {}", display_name(&path), e)),
        }
    }

    Ok(report)
}

/// One import, start to finish.
///
/// The order — assemble, insert the row, move the files into `<store>/<id>` —
/// is deliberate and is documented on `local_create`: a crash between the row
/// and the files leaves a mod that deploys nothing and says so, while the
/// opposite order leaves a directory nothing owns and nothing ever cleans up.
#[allow(clippy::too_many_arguments)]
fn import_one(
    state: &AppState,
    store_root: &std::path::Path,
    path: &std::path::Path,
    opts: &ImportOptions<'_>,
    app_id: Option<i64>,
    slug: Option<&str>,
    origin: Origin,
    over: Option<&ImportOverride>,
) -> AppResult<LocalMod> {
    let prepared = store::prepare(store_root, path, opts)?;

    let source = prepared.metadata.as_ref().and_then(|m| {
        m.item_ref(tmc_core::api::api_base())
            .map(|(kind, item_id)| SourceRef {
                kind,
                item_id,
                release_id: m.release_id,
                web_url: m.web_url.clone(),
            })
    });

    let name = over
        .and_then(|o| o.name.clone())
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| prepared.name.clone());

    let created = state.library.local_create(&NewLocalMod {
        name,
        app_id,
        app_slug: slug.map(str::to_owned),
        version: prepared.metadata.as_ref().and_then(|m| m.version.clone()),
        author: prepared.metadata.as_ref().and_then(|m| m.author.clone()),
        origin,
        origin_label: Some(display_name(path)),
        rel_path: prepared.rel_path.clone(),
        source,
        files: prepared.files as i64,
        bytes: prepared.bytes as i64,
    });

    let id = match created {
        Ok(id) => id,
        Err(e) => {
            store::discard(store_root);

            return Err(e);
        }
    };

    if let Err(e) = store::commit(store_root, &prepared, id) {
        // The row is only useful with its files. Rolling it back is what stops
        // a failed import leaving a mod that appears in every list and
        // contributes nothing.
        let _ = state.library.local_delete(id);

        return Err(e);
    }

    audit!(
        state.audit,
        Info,
        Install,
        "local.import",
        format!(
            "Imported {} ({} file(s)) → {}",
            prepared.name,
            prepared.files,
            if prepared.rel_path.is_empty() {
                "the game's folder".to_string()
            } else {
                prepared.rel_path.clone()
            }
        )
    );

    state
        .library
        .local_get(id)?
        .ok_or_else(|| AppError::internal("the imported mod vanished"))
}

// -------------------------------------------------------------- Local mod rows

#[tauri::command]
pub fn local_list(state: State<'_, AppState>, app_id: Option<i64>) -> AppResult<Vec<LocalMod>> {
    state.library.local_list(app_id)
}

#[tauri::command]
pub fn local_get(state: State<'_, AppState>, id: i64) -> AppResult<Option<LocalMod>> {
    state.library.local_get(id)
}

/// Rename it, re-version it, note something about it, or move where it lands.
///
/// Changing `relPath` MOVES the files, which is why it is not just a column
/// write: a mod whose row says `BepInEx/plugins` and whose files sit under
/// `mods` deploys to the wrong place, and nothing downstream would notice.
#[tauri::command]
pub fn local_patch(state: State<'_, AppState>, id: i64, patch: LocalPatch) -> AppResult<LocalMod> {
    let current = state
        .library
        .local_get(id)?
        .ok_or_else(|| AppError::invalid("That imported mod is not on this device."))?;

    if let Some(requested) = &patch.rel_path {
        let cleaned = store::sanitise_rel(requested)?;

        if cleaned != current.rel_path {
            store::relayout(
                &state.paths.local_mods_dir(),
                id,
                &current.rel_path,
                &cleaned,
            )?;

            audit!(
                state.audit,
                Info,
                Install,
                "local.relayout",
                format!(
                    "{} now installs to {}",
                    current.name,
                    if cleaned.is_empty() {
                        "the game's folder"
                    } else {
                        cleaned.as_str()
                    }
                )
            );
        }
    }

    state.library.local_patch(id, &patch)
}

/// Forget an imported mod and delete its files.
///
/// Returns the sandboxes it was in, because they now hold files in a game
/// folder that nothing will ever remove unless one of them is redeployed —
/// and a purge before the delete is what actually takes them out. The UI is
/// what decides whether to offer that, so this reports rather than acts.
#[tauri::command]
pub fn local_delete(state: State<'_, AppState>, id: i64) -> AppResult<Vec<i64>> {
    let Some(row) = state.library.local_get(id)? else {
        return Ok(Vec::new());
    };

    let affected = state.library.local_delete(id)?;

    store::remove(&state.paths.local_mods_dir(), id);

    audit!(
        state.audit,
        Info,
        Install,
        "local.delete",
        format!("Removed the imported mod {}", row.name)
    );

    Ok(affected)
}

/// Put an imported mod in a sandbox.
#[tauri::command]
pub fn local_add_to_sandbox(
    state: State<'_, AppState>,
    sandbox_id: i64,
    id: i64,
) -> AppResult<String> {
    let sandbox = state
        .library
        .sandbox_get(sandbox_id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let row = state
        .library
        .local_get(id)?
        .ok_or_else(|| AppError::invalid("That imported mod is not on this device."))?;

    if row.app_id.is_some_and(|app| app != sandbox.app_id) {
        return Err(AppError::invalid(
            "That mod was imported for a different game than this sandbox.",
        ));
    }

    state.library.sandbox_add_local(sandbox_id, &row)
}

// ------------------------------------------------------------------- Adopting

/// One thing in the game folder that the app cannot account for.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptCandidate {
    pub token: String,
    #[serde(flatten)]
    pub found: adopt::Candidate,
}

/// What is already in this sandbox's game folder that nothing here put there.
///
/// The answer to "somebody has been modding for years and this app knows about
/// none of it". Reads only the folders the game's own rules declare as mod
/// targets — see [`tmc_core::local::adopt`] for why that bound is the whole
/// difference between this and a filesystem scan.
#[tauri::command]
pub fn import_unmanaged(
    state: State<'_, AppState>,
    sandbox_id: i64,
) -> AppResult<Vec<AdoptCandidate>> {
    let sandbox = state
        .library
        .sandbox_get(sandbox_id)?
        .ok_or_else(|| AppError::invalid("That sandbox does not exist."))?;

    let settings = state.settings.get();
    let game_dir = tmc_core::library::deploy::target_dir(&sandbox, &settings)?;

    let plugins = state.app_plugins();

    let targets = sandbox
        .app_slug
        .as_deref()
        .map(|s| plugins.mod_targets(s))
        .unwrap_or_default();

    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let found = adopt::scan(
        &state.library,
        &adopt::AdoptScan {
            game_dir: &game_dir,
            mod_targets: &targets,
            app_id: Some(sandbox.app_id),
            local_store: &state.paths.local_mods_dir(),
        },
    )?;

    Ok(found
        .into_iter()
        .map(|c| AdoptCandidate {
            token: state.vault.mint(c.path.clone()),
            found: c,
        })
        .collect())
}

// ------------------------------------------------------------------ Managers

/// One mod manager this build can read, and whether it found anything.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerInfo {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub notes: Vec<String>,
    /// The TMC games this descriptor can map, so the UI can say "this one
    /// covers Valheim and V Rising" rather than offering a manager that could
    /// never match anything the user has.
    pub slugs: Vec<String>,
}

/// One mod another manager holds, ready to import.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerCandidate {
    pub token: String,
    #[serde(flatten)]
    pub found: managers::ManagerMod,
}

/// Every manager descriptor: the ones compiled in, plus the user's own.
#[tauri::command]
pub fn import_managers(state: State<'_, AppState>) -> Vec<ManagerInfo> {
    manager_specs(&state)
        .into_iter()
        .map(|spec| {
            let mut slugs: Vec<String> = spec.games.iter().map(|g| g.slug.clone()).collect();

            slugs.sort();
            slugs.dedup();

            ManagerInfo {
                id: spec.id,
                label: spec.label,
                homepage: spec.homepage,
                notes: spec.notes,
                slugs,
            }
        })
        .collect()
}

/// Read one manager's own storage.
///
/// Nothing there is modified: the scan lists directories and reads the
/// manager's per-mod JSON where it declares one. Importing a result COPIES it.
#[tauri::command]
pub fn import_manager_scan(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
) -> AppResult<Vec<ManagerCandidate>> {
    let spec = manager_specs(&state)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| AppError::invalid("There is no mod manager plugin with that id."))?;

    let roots = state.detect_roots(&app);

    let found = managers::scan(&spec, &roots);

    audit!(
        state.audit,
        Info,
        Plugin,
        "manager.scan",
        format!("Read {}: {} mod(s) found", spec.label, found.len()),
        plugin = spec.id
    );

    Ok(found
        .into_iter()
        .map(|m| ManagerCandidate {
            token: state.vault.mint(m.path.clone()),
            found: m,
        })
        .collect())
}

/// Both sources of manager descriptors, user's winning on id.
///
/// Compiled-in and `<plugins>/manager/` come from [`managers::resolve`]; an
/// installed registry bundle carrying a `manager` block is the third, and it
/// goes through the registry's own approval and signature gate on the way out
/// — which is the entire reason a bundle is a different thing from a file
/// dropped in the plugins folder.
fn manager_specs(state: &AppState) -> Vec<ManagerSpec> {
    let (mut specs, errors) = managers::resolve(&state.paths.plugins);

    for (source, error) in errors {
        tracing::warn!("manager plugin {source} did not load: {error}");
    }

    for record in state.plugins.list() {
        if !record.enabled || !record.kinds.iter().any(|k| k == "manager") {
            continue;
        }

        // `active` re-verifies the fingerprint and the signature gate rather
        // than trusting the stored record — the same rule every other use of a
        // registry plugin follows.
        let Ok(manifest) = state.plugins.active(&record.id) else {
            continue;
        };

        if let Some(spec) = manifest.manager {
            if let Some(slot) = specs.iter_mut().find(|s| s.id == spec.id) {
                *slot = spec;
            } else {
                specs.push(spec);
            }
        }
    }

    specs
}

// ------------------------------------------------------------------- Helpers

fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("that file")
        .to_string()
}

/// Turn a batch of paths the OS handed us into a batch the webview can name.
///
/// Called from the window's drag-drop handler in `lib.rs`, which is the only
/// place a path enters this subsystem.
pub fn batch_from_drop(state: &AppState, paths: Vec<PathBuf>) -> DropBatch {
    let files = paths
        .into_iter()
        .filter(|p| is_importable(p))
        .take(512)
        .map(|path| {
            let meta = std::fs::metadata(&path).ok();

            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("dropped file")
                .to_string();

            let lower = name.to_ascii_lowercase();

            PendingFile {
                token: state.vault.mint(path),
                name,
                is_dir,
                bytes: meta.filter(|m| m.is_file()).map(|m| m.len()).unwrap_or(0),
                is_archive: lower.ends_with(".zip")
                    || lower.ends_with(".tar")
                    || lower.ends_with(".tar.gz")
                    || lower.ends_with(".tgz"),
            }
        })
        .collect();

    DropBatch {
        at: tmc_core::session::now_ms(),
        files,
    }
}
