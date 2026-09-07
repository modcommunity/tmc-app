//! **Sandboxes** — a named set of mods, a deployment strategy and a launch
//! configuration, all pointed at one game.
//!
//! The thing other managers call a profile (Mod Organizer, Vortex) or an
//! instance (CurseForge, r2modman). A user has several per game — "vanilla-ish",
//! "the one my friends use", "my server" — and switching between them is
//! supposed to be instant and non-destructive.
//!
//! WHAT LIVES WHERE, AND WHY
//! -------------------------
//! A sandbox is split across two stores on purpose, by the same test the app's
//! settings use — *would this be wrong to apply on a different machine?*
//!
//! | Fact | Where | Because |
//! | --- | --- | --- |
//! | name, mods, order, options, launch flags | the account (`AppInstall`) | signing in on a second machine should reproduce it |
//! | which folder it deploys into | here | that path exists on one machine |
//! | which release of each mod is staged | here | so does that |
//! | the deployment ledger | here | it describes files on one disk |
//!
//! The cloud half is optional. `cloud_sync` off means the sandbox is never sent
//! anywhere and lives only in this database — which is the whole answer to
//! "I do not want my mod list on your server", and it has to be a per-sandbox
//! switch rather than a global one because the useful case is "sync my
//! Minecraft profiles, not the one I use for testing".
//!
//! A sandbox with `cloud_sync` off is not second-class: everything in this
//! module works identically for one, and `remote_id` being `NULL` is the only
//! difference.
//!
//! THE ENVIRONMENT
//! ---------------
//! `client`, `server` or `shared`. It is not decoration — it decides which mods
//! are even offered (a server-side-only mod in a client sandbox does nothing),
//! which options are shown, and which presets apply. A dedicated server and a
//! player's game are the same files arranged differently, and a manager that
//! cannot tell them apart offers everybody both halves of every list.

use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::deploy::{LedgerEntry, Strategy};
use crate::error::{AppError, AppResult};
use crate::logging::now_rfc3339;

use super::db::LibraryDb;

/// What a sandbox is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Environment {
    /// The default: most sandboxes are somebody's game, not somebody's server.
    #[default]
    Client,
    Server,
    /// Both — the sandbox holds mods that a client and a server each need, and
    /// is deployed to whichever the user launches.
    Shared,
}

impl Environment {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Server => "server",
            Self::Shared => "shared",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.to_ascii_lowercase().as_str() {
            "client" => Some(Self::Client),
            "server" => Some(Self::Server),
            "shared" => Some(Self::Shared),
            _ => None,
        }
    }

    /// Does a sandbox in this environment want an item declared for `other`?
    ///
    /// `shared` accepts everything and is accepted by everything, which is what
    /// the website's own `ALL` environment means on a mod — most mods run on
    /// both sides and saying so should not exclude them from either.
    pub fn accepts(self, other: Environment) -> bool {
        self == Environment::Shared || other == Environment::Shared || self == other
    }
}

/// One mod or asset in a sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxMod {
    /// `mod:1234` — the identity the merge tree and the ledger use. Derived
    /// from `(kind, item_id)` and stable across a rename of the item.
    pub mod_key: String,
    pub kind: String,
    pub item_id: i64,
    pub name: String,

    /// Off means "keep it in the list but do not deploy it". The standard mod
    /// manager toggle, and the reason removing an item is not the only way to
    /// stop using it.
    pub enabled: bool,

    /// Higher wins a contested path. Ties break by `mod_key`.
    pub priority: i64,

    /// Which release is in this sandbox's staging folder, if any. Per-sandbox
    /// rather than per-subscription because two sandboxes for one game
    /// routinely want different versions of the same mod — that is most of the
    /// point of having two.
    pub release_id: Option<i64>,
    pub version: Option<String>,
    pub staged_at: Option<String>,
    pub last_error: Option<String>,
}

impl SandboxMod {
    /// The key for a content item.
    pub fn key_for(kind: &str, item_id: i64) -> String {
        format!("{kind}:{item_id}")
    }
}

/// A sandbox, as the app holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sandbox {
    pub id: i64,
    /// `AppInstall.id`, or `None` for a sandbox kept off the cloud.
    pub remote_id: Option<i64>,

    pub app_id: i64,
    pub app_slug: Option<String>,
    pub app_name: Option<String>,

    pub name: String,
    pub description: Option<String>,

    pub environment: Environment,
    pub strategy: Strategy,

    pub game_version: Option<String>,
    pub loader: Option<String>,
    /// The `sandbox.json` preset this was created from, for the UI to show.
    pub preset: Option<String>,

    pub is_default: bool,
    pub cloud_sync: bool,

    /// Keep this sandbox's mods at the newest release its subscriptions offer.
    ///
    /// Per sandbox rather than global because the useful case is exactly the
    /// split: a "current" profile that tracks the latest, and a pinned one for
    /// the modpack somebody's friends are all running.
    pub auto_update: bool,

    /// The game folder on THIS machine. `None` means the app's configured
    /// directory for this game.
    pub game_dir: Option<String>,

    pub options: BTreeMap<String, serde_json::Value>,
    pub launch_args: Vec<String>,
    pub launch_env: BTreeMap<String, String>,

    pub deployed_at: Option<String>,
    /// The last deploy's report. Advisory, for the UI.
    pub last_deploy: Option<serde_json::Value>,

    pub created_at: String,
    pub updated_at: String,

    pub mods: Vec<SandboxMod>,
}

impl Sandbox {
    pub fn enabled_mods(&self) -> impl Iterator<Item = &SandboxMod> {
        self.mods.iter().filter(|m| m.enabled)
    }

    /// Has anything changed since the last deploy?
    ///
    /// Cheap and deliberately conservative — it compares the mod set against
    /// what the ledger says was deployed, not the files themselves. A false
    /// "needs deploying" costs a no-op redeploy that reuses everything; a false
    /// "up to date" costs somebody an evening wondering why their new mod is
    /// not loading.
    pub fn needs_deploy(&self, ledger: &[LedgerEntry]) -> bool {
        if self.deployed_at.is_none() {
            return self.enabled_mods().next().is_some();
        }

        let deployed: std::collections::HashSet<&str> =
            ledger.iter().map(|e| e.mod_key.as_str()).collect();

        let wanted: std::collections::HashSet<&str> =
            self.enabled_mods().map(|m| m.mod_key.as_str()).collect();

        /*
         * A mod that is enabled and staged but contributed no files is not a
         * difference — some mods legitimately ship nothing the merge tree can
         * see until they are configured. Comparing only in the direction
         * "wanted but never deployed" would then re-deploy forever, so the
         * comparison is symmetric and an empty mod is filtered out by having no
         * staged release rather than by being absent from the ledger.
         */
        wanted
            .iter()
            .any(|key| !deployed.contains(key) && self.is_staged(key))
            || deployed.iter().any(|key| !wanted.contains(key))
    }

    fn is_staged(&self, key: &str) -> bool {
        self.mods
            .iter()
            .any(|m| m.mod_key == key && m.staged_at.is_some())
    }
}

/// What creating a sandbox needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewSandbox {
    pub app_id: i64,
    #[serde(default)]
    pub app_slug: Option<String>,
    #[serde(default)]
    pub app_name: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub environment: Environment,
    #[serde(default)]
    pub strategy: Strategy,
    #[serde(default)]
    pub game_version: Option<String>,
    #[serde(default)]
    pub loader: Option<String>,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default)]
    pub game_dir: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
    /// Default ON: a user who has not thought about it is better served by
    /// their sandboxes surviving a reinstall than by them not leaving the
    /// machine, and the switch is one click away on every sandbox.
    #[serde(default = "yes")]
    pub cloud_sync: bool,
    /// Default ON. Somebody who has not thought about it is better served by a
    /// mod that stays current than by one that silently rots.
    #[serde(default = "yes")]
    pub auto_update: bool,
}

fn yes() -> bool {
    true
}

/// A partial update. Every field absent means "leave it".
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SandboxPatch {
    pub name: Option<String>,
    /// Nested `Option` so `null` (clear it) and absent (leave it) are different
    /// requests — which they are for every nullable column here.
    #[serde(default, deserialize_with = "present")]
    pub description: Option<Option<String>>,
    pub environment: Option<Environment>,
    pub strategy: Option<Strategy>,
    #[serde(default, deserialize_with = "present")]
    pub game_version: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub loader: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub game_dir: Option<Option<String>>,
    pub options: Option<BTreeMap<String, serde_json::Value>>,
    pub launch_args: Option<Vec<String>>,
    pub launch_env: Option<BTreeMap<String, String>>,
    pub cloud_sync: Option<bool>,
    pub is_default: Option<bool>,
    pub auto_update: Option<bool>,
}

/// Deserialise a nullable field into `Some(_)` whenever the key was PRESENT,
/// including when its value was `null`.
///
/// `Option<Option<T>>` is the only way to tell "clear the loader" from "do not
/// mention the loader" over JSON, and serde's own `Option` handling collapses
/// both into `None`. This is the standard fix: the field's `default` supplies
/// the absent case, and this function only ever runs when a value was actually
/// there.
fn present<'de, T, D>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    T::deserialize(deserializer).map(Some)
}

/// Cap on sandboxes per game. Matches the server's `MAX_INSTALLS_PER_APP`, so a
/// local sandbox that later syncs is not refused on its way up.
pub const MAX_SANDBOXES_PER_APP: usize = 50;

/// Cap on mods in one sandbox.
pub const MAX_MODS_PER_SANDBOX: usize = 5_000;

impl LibraryDb {
    // ------------------------------------------------------------- Creation

    pub fn sandbox_create(&self, new: &NewSandbox) -> AppResult<i64> {
        let name = new.name.trim();

        if name.is_empty() || name.len() > 64 {
            return Err(AppError::invalid("A sandbox needs a name."));
        }

        if self.sandbox_count(new.app_id)? >= MAX_SANDBOXES_PER_APP {
            return Err(AppError::invalid(format!(
                "You already have {MAX_SANDBOXES_PER_APP} sandboxes for this game."
            )));
        }

        let now = now_rfc3339();
        let first = self.sandbox_count(new.app_id)? == 0;

        let id = self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO sandbox (
                    app_id, app_slug, app_name, name, description,
                    environment, strategy, game_version, loader, preset,
                    is_default, cloud_sync, auto_update, game_dir, options,
                    created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?16
                )
                "#,
                params![
                    new.app_id,
                    new.app_slug,
                    new.app_name,
                    name,
                    new.description,
                    new.environment.as_str(),
                    new.strategy.as_str(),
                    new.game_version,
                    new.loader,
                    new.preset,
                    // The first sandbox for a game is the default whether or
                    // not anyone asked, so a user with exactly one never has
                    // none selected.
                    first as i64,
                    new.cloud_sync as i64,
                    new.auto_update as i64,
                    new.game_dir,
                    serde_json::to_string(&new.options).unwrap_or_else(|_| "{}".into()),
                    now,
                ],
            )?;

            Ok(conn.last_insert_rowid())
        })?;

        Ok(id)
    }

    pub fn sandbox_count(&self, app_id: i64) -> AppResult<usize> {
        self.with(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sandbox WHERE app_id = ?1",
                params![app_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
        })
    }

    // -------------------------------------------------------------- Reading

    pub fn sandbox_list(&self, app_id: Option<i64>) -> AppResult<Vec<Sandbox>> {
        let mut sandboxes: Vec<Sandbox> = self.with(|conn| {
            let mut stmt = conn.prepare(&format!(
                "{SANDBOX_SELECT} {} ORDER BY is_default DESC, name ASC",
                if app_id.is_some() {
                    "WHERE app_id = ?1"
                } else {
                    ""
                }
            ))?;

            let rows = match app_id {
                Some(id) => stmt.query_map(params![id], row_to_sandbox)?.collect(),
                None => stmt.query_map([], row_to_sandbox)?.collect(),
            };

            rows
        })?;

        for sandbox in &mut sandboxes {
            sandbox.mods = self.sandbox_mods(sandbox.id)?;
        }

        Ok(sandboxes)
    }

    pub fn sandbox_get(&self, id: i64) -> AppResult<Option<Sandbox>> {
        let found = self.with(|conn| {
            conn.prepare(&format!("{SANDBOX_SELECT} WHERE id = ?1"))?
                .query_row(params![id], row_to_sandbox)
                .optional()
        })?;

        let Some(mut sandbox) = found else {
            return Ok(None);
        };

        sandbox.mods = self.sandbox_mods(id)?;

        Ok(Some(sandbox))
    }

    pub fn sandbox_by_remote(&self, remote_id: i64) -> AppResult<Option<i64>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT id FROM sandbox WHERE remote_id = ?1",
                params![remote_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()
        })
    }

    /// The default sandbox for a game, if there is one.
    pub fn sandbox_default_for(&self, app_id: i64) -> AppResult<Option<i64>> {
        self.with(|conn| {
            conn.query_row(
                "SELECT id FROM sandbox WHERE app_id = ?1 AND is_default = 1",
                params![app_id],
                |r| r.get::<_, i64>(0),
            )
            .optional()
        })
    }

    // -------------------------------------------------------------- Writing

    pub fn sandbox_patch(&self, id: i64, patch: &SandboxPatch) -> AppResult<()> {
        if let Some(name) = &patch.name {
            if name.trim().is_empty() || name.len() > 64 {
                return Err(AppError::invalid("A sandbox needs a name."));
            }
        }

        let now = now_rfc3339();

        self.with(|conn| {
            /*
             * One statement with `COALESCE`-style guards rather than a built
             * SQL string. Every parameter is bound, and a field the caller did
             * not mention keeps its stored value because the `?is_set` flag
             * beside it is false — which is the only way to express "leave it"
             * without either building SQL or reading-then-writing the whole row
             * and losing a concurrent change.
             */
            conn.execute(
                r#"
                UPDATE sandbox SET
                    name         = CASE WHEN ?2  THEN ?3  ELSE name         END,
                    description  = CASE WHEN ?4  THEN ?5  ELSE description  END,
                    environment  = CASE WHEN ?6  THEN ?7  ELSE environment  END,
                    strategy     = CASE WHEN ?8  THEN ?9  ELSE strategy     END,
                    game_version = CASE WHEN ?10 THEN ?11 ELSE game_version END,
                    loader       = CASE WHEN ?12 THEN ?13 ELSE loader       END,
                    game_dir     = CASE WHEN ?14 THEN ?15 ELSE game_dir     END,
                    options      = CASE WHEN ?16 THEN ?17 ELSE options      END,
                    launch_args  = CASE WHEN ?18 THEN ?19 ELSE launch_args  END,
                    launch_env   = CASE WHEN ?20 THEN ?21 ELSE launch_env   END,
                    cloud_sync   = CASE WHEN ?22 THEN ?23 ELSE cloud_sync   END,
                    auto_update  = CASE WHEN ?25 THEN ?26 ELSE auto_update  END,
                    updated_at   = ?24
                WHERE id = ?1
                "#,
                params![
                    id,
                    patch.name.is_some(),
                    patch.name,
                    patch.description.is_some(),
                    patch.description.clone().flatten(),
                    patch.environment.is_some(),
                    patch.environment.map(|e| e.as_str()),
                    patch.strategy.is_some(),
                    patch.strategy.map(|s| s.as_str()),
                    patch.game_version.is_some(),
                    patch.game_version.clone().flatten(),
                    patch.loader.is_some(),
                    patch.loader.clone().flatten(),
                    patch.game_dir.is_some(),
                    patch.game_dir.clone().flatten(),
                    patch.options.is_some(),
                    patch
                        .options
                        .as_ref()
                        .map(|o| serde_json::to_string(o).unwrap_or_else(|_| "{}".into())),
                    patch.launch_args.is_some(),
                    patch
                        .launch_args
                        .as_ref()
                        .map(|a| serde_json::to_string(a).unwrap_or_else(|_| "[]".into())),
                    patch.launch_env.is_some(),
                    patch
                        .launch_env
                        .as_ref()
                        .map(|e| serde_json::to_string(e).unwrap_or_else(|_| "{}".into())),
                    patch.cloud_sync.is_some(),
                    patch.cloud_sync,
                    now,
                    patch.auto_update.is_some(),
                    patch.auto_update,
                ],
            )?;

            Ok(())
        })?;

        if patch.is_default == Some(true) {
            self.sandbox_set_default(id)?;
        }

        Ok(())
    }

    /// Make this the pre-selected sandbox for its game, and no other.
    ///
    /// Both statements in one transaction: two clients racing would otherwise
    /// leave a game with two defaults or none, and "none" means the launch
    /// button on the game's page has nothing to launch.
    pub fn sandbox_set_default(&self, id: i64) -> AppResult<()> {
        self.with(|conn| {
            let app_id: i64 = conn.query_row(
                "SELECT app_id FROM sandbox WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )?;

            let tx = conn.unchecked_transaction()?;

            tx.execute(
                "UPDATE sandbox SET is_default = 0 WHERE app_id = ?1",
                params![app_id],
            )?;
            tx.execute(
                "UPDATE sandbox SET is_default = 1 WHERE id = ?1",
                params![id],
            )?;

            tx.commit()?;

            Ok(())
        })
    }

    pub fn sandbox_delete(&self, id: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute("DELETE FROM sandbox WHERE id = ?1", params![id])?;

            Ok(())
        })
    }

    /// Record a completed deploy.
    pub fn sandbox_mark_deployed(&self, id: i64, report: &serde_json::Value) -> AppResult<()> {
        let now = now_rfc3339();

        self.with(|conn| {
            conn.execute(
                "UPDATE sandbox SET deployed_at = ?2, last_deploy = ?3, updated_at = ?2
                 WHERE id = ?1",
                params![id, now, report.to_string()],
            )?;

            Ok(())
        })
    }

    /// Record that a sandbox is no longer deployed.
    pub fn sandbox_mark_purged(&self, id: i64) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sandbox SET deployed_at = NULL, updated_at = ?2 WHERE id = ?1",
                params![id, now_rfc3339()],
            )?;

            Ok(())
        })
    }

    // ----------------------------------------------------------------- Mods

    pub fn sandbox_mods(&self, sandbox_id: i64) -> AppResult<Vec<SandboxMod>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT mod_key, kind, item_id, name, enabled, priority,
                        release_id, version, staged_at, last_error
                 FROM sandbox_mod WHERE sandbox_id = ?1
                 ORDER BY priority ASC, mod_key ASC",
            )?;

            let rows: rusqlite::Result<Vec<SandboxMod>> = stmt
                .query_map(params![sandbox_id], |row| {
                    Ok(SandboxMod {
                        mod_key: row.get(0)?,
                        kind: row.get(1)?,
                        item_id: row.get(2)?,
                        name: row.get(3)?,
                        enabled: row.get::<_, i64>(4)? != 0,
                        priority: row.get(5)?,
                        release_id: row.get(6)?,
                        version: row.get(7)?,
                        staged_at: row.get(8)?,
                        last_error: row.get(9)?,
                    })
                })?
                .collect();

            rows
        })
    }

    /// Add an item, or update the parts of it a caller may set.
    ///
    /// The staging columns are NOT touched here, for the same reason
    /// `upsert_remote` does not touch the install columns: a cloud sync must
    /// not forget that a mod is already downloaded.
    pub fn sandbox_add_mod(
        &self,
        sandbox_id: i64,
        kind: &str,
        item_id: i64,
        name: &str,
    ) -> AppResult<String> {
        let key = SandboxMod::key_for(kind, item_id);

        let count: usize = self.with(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM sandbox_mod WHERE sandbox_id = ?1",
                params![sandbox_id],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n as usize)
        })?;

        if count >= MAX_MODS_PER_SANDBOX {
            return Err(AppError::invalid(format!(
                "A sandbox can hold at most {MAX_MODS_PER_SANDBOX} items."
            )));
        }

        self.with(|conn| {
            // A new item goes to the END of the load order, which is what
            // "later wins" means and what every other manager does.
            let next: i64 = conn.query_row(
                "SELECT COALESCE(MAX(priority), -1) + 1 FROM sandbox_mod WHERE sandbox_id = ?1",
                params![sandbox_id],
                |r| r.get(0),
            )?;

            conn.execute(
                r#"
                INSERT INTO sandbox_mod (
                    sandbox_id, mod_key, kind, item_id, name, enabled, priority
                ) VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
                ON CONFLICT(sandbox_id, mod_key) DO UPDATE SET
                    name = excluded.name,
                    kind = excluded.kind,
                    item_id = excluded.item_id
                "#,
                params![sandbox_id, key, kind, item_id, name, next],
            )?;

            Ok(())
        })?;

        Ok(key)
    }

    pub fn sandbox_set_mod_enabled(
        &self,
        sandbox_id: i64,
        mod_key: &str,
        enabled: bool,
    ) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sandbox_mod SET enabled = ?3 WHERE sandbox_id = ?1 AND mod_key = ?2",
                params![sandbox_id, mod_key, enabled as i64],
            )?;

            Ok(())
        })
    }

    pub fn sandbox_remove_mod(&self, sandbox_id: i64, mod_key: &str) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "DELETE FROM sandbox_mod WHERE sandbox_id = ?1 AND mod_key = ?2",
                params![sandbox_id, mod_key],
            )?;

            Ok(())
        })
    }

    /// Rewrite the load order from a list of keys, first to last.
    ///
    /// Keys the sandbox does not hold are ignored, and keys the list omits keep
    /// their place after everything named — a UI that drags one row should not
    /// have to send the whole list correctly to avoid scrambling the rest.
    pub fn sandbox_reorder(&self, sandbox_id: i64, keys: &[String]) -> AppResult<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;

            for (index, key) in keys.iter().enumerate() {
                tx.execute(
                    "UPDATE sandbox_mod SET priority = ?3
                     WHERE sandbox_id = ?1 AND mod_key = ?2",
                    params![sandbox_id, key, index as i64],
                )?;
            }

            // Anything not named keeps a priority above every named entry.
            tx.execute(
                "UPDATE sandbox_mod SET priority = priority + ?2
                 WHERE sandbox_id = ?1 AND priority >= ?2",
                params![sandbox_id, keys.len() as i64],
            )?;

            tx.commit()?;

            Ok(())
        })
    }

    /// Record that a release is now in this sandbox's staging folder.
    pub fn sandbox_mark_staged(
        &self,
        sandbox_id: i64,
        mod_key: &str,
        release_id: Option<i64>,
        version: Option<&str>,
    ) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sandbox_mod
                 SET release_id = ?3, version = ?4, staged_at = ?5, last_error = NULL
                 WHERE sandbox_id = ?1 AND mod_key = ?2",
                params![sandbox_id, mod_key, release_id, version, now_rfc3339()],
            )?;

            Ok(())
        })
    }

    pub fn sandbox_mark_stage_failed(
        &self,
        sandbox_id: i64,
        mod_key: &str,
        error: &str,
    ) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sandbox_mod SET last_error = ?3 WHERE sandbox_id = ?1 AND mod_key = ?2",
                params![sandbox_id, mod_key, error],
            )?;

            Ok(())
        })
    }

    /// Forget that a mod is staged — after its staging folder is deleted.
    pub fn sandbox_clear_staged(&self, sandbox_id: i64, mod_key: &str) -> AppResult<()> {
        self.with(|conn| {
            conn.execute(
                "UPDATE sandbox_mod
                 SET release_id = NULL, version = NULL, staged_at = NULL
                 WHERE sandbox_id = ?1 AND mod_key = ?2",
                params![sandbox_id, mod_key],
            )?;

            Ok(())
        })
    }

    // --------------------------------------------------------------- Ledger

    pub fn sandbox_ledger(&self, sandbox_id: i64) -> AppResult<Vec<LedgerEntry>> {
        self.with(|conn| {
            let mut stmt = conn.prepare(
                "SELECT path, kind, mod_key, source, size, mtime_ms, backup
                 FROM deployment WHERE sandbox_id = ?1",
            )?;

            let rows: rusqlite::Result<Vec<LedgerEntry>> = stmt
                .query_map(params![sandbox_id], |row| {
                    Ok(LedgerEntry {
                        path: row.get(0)?,
                        kind: row.get(1)?,
                        mod_key: row.get(2)?,
                        source: row.get(3)?,
                        size: row.get::<_, i64>(4)? as u64,
                        mtime_ms: row.get(5)?,
                        backup: row.get(6)?,
                    })
                })?
                .collect();

            rows
        })
    }

    /// Replace the ledger wholesale.
    ///
    /// One transaction, and it must be: a ledger half-written by a crash names
    /// some of the files in somebody's game folder and not the others, and the
    /// ones it does not name can never be cleaned up.
    pub fn sandbox_set_ledger(&self, sandbox_id: i64, entries: &[LedgerEntry]) -> AppResult<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;

            tx.execute(
                "DELETE FROM deployment WHERE sandbox_id = ?1",
                params![sandbox_id],
            )?;

            {
                let mut stmt = tx.prepare(
                    "INSERT INTO deployment
                        (sandbox_id, path, kind, mod_key, source, size, mtime_ms, backup)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )?;

                for entry in entries {
                    stmt.execute(params![
                        sandbox_id,
                        entry.path,
                        entry.kind,
                        entry.mod_key,
                        entry.source,
                        entry.size as i64,
                        entry.mtime_ms,
                        entry.backup,
                    ])?;
                }
            }

            tx.commit()?;

            Ok(())
        })
    }

    // ------------------------------------------------------- Cloud mirroring

    /// Create or update the sandbox backing a cloud install.
    ///
    /// The cloud owns the DEFINITION; this device owns `game_dir` and the
    /// staging state, so neither is written here. A sandbox the user turned
    /// `cloud_sync` off on is left alone entirely — turning the switch off has
    /// to mean the server stops being the authority, or it is not a switch.
    #[allow(clippy::too_many_arguments)]
    pub fn sandbox_upsert_remote(&self, remote: &RemoteSandbox<'_>) -> AppResult<i64> {
        let now = now_rfc3339();

        if let Some(id) = self.sandbox_by_remote(remote.remote_id)? {
            let synced: bool = self.with(|conn| {
                conn.query_row(
                    "SELECT cloud_sync FROM sandbox WHERE id = ?1",
                    params![id],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n != 0)
            })?;

            if !synced {
                return Ok(id);
            }

            self.with(|conn| {
                conn.execute(
                    r#"
                    UPDATE sandbox SET
                        app_id = ?2, app_slug = ?3, app_name = ?4, name = ?5,
                        description = ?6, environment = ?7, strategy = ?8,
                        game_version = ?9, loader = ?10, is_default = ?11,
                        options = ?12, launch_args = ?13, launch_env = ?14,
                        updated_at = ?15
                    WHERE id = ?1
                    "#,
                    params![
                        id,
                        remote.app_id,
                        remote.app_slug,
                        remote.app_name,
                        remote.name,
                        remote.description,
                        remote.environment.as_str(),
                        remote.strategy.as_str(),
                        remote.game_version,
                        remote.loader,
                        remote.is_default as i64,
                        remote.options,
                        remote.launch_args,
                        remote.launch_env,
                        now,
                    ],
                )?;

                Ok(())
            })?;

            self.sandbox_replace_mods(id, remote.items)?;

            return Ok(id);
        }

        let id = self.with(|conn| {
            conn.execute(
                r#"
                INSERT INTO sandbox (
                    remote_id, app_id, app_slug, app_name, name, description,
                    environment, strategy, game_version, loader, is_default,
                    cloud_sync, options, launch_args, launch_env,
                    created_at, updated_at
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?13, ?14, ?15, ?15
                )
                "#,
                params![
                    remote.remote_id,
                    remote.app_id,
                    remote.app_slug,
                    remote.app_name,
                    remote.name,
                    remote.description,
                    remote.environment.as_str(),
                    remote.strategy.as_str(),
                    remote.game_version,
                    remote.loader,
                    remote.is_default as i64,
                    remote.options,
                    remote.launch_args,
                    remote.launch_env,
                    now,
                ],
            )?;

            Ok(conn.last_insert_rowid())
        })?;

        self.sandbox_replace_mods(id, remote.items)?;

        Ok(id)
    }

    /// Apply the cloud's item list, keeping every device-local staging column.
    fn sandbox_replace_mods(&self, id: i64, items: &[RemoteItem]) -> AppResult<()> {
        self.with(|conn| {
            let tx = conn.unchecked_transaction()?;

            let keys: Vec<String> = items.iter().map(|i| i.mod_key()).collect();

            {
                let mut stmt = tx.prepare(
                    r#"
                    INSERT INTO sandbox_mod (
                        sandbox_id, mod_key, kind, item_id, name, enabled, priority
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    ON CONFLICT(sandbox_id, mod_key) DO UPDATE SET
                        kind = excluded.kind,
                        item_id = excluded.item_id,
                        name = excluded.name,
                        enabled = excluded.enabled,
                        priority = excluded.priority
                    "#,
                )?;

                for item in items {
                    stmt.execute(params![
                        id,
                        item.mod_key(),
                        item.kind,
                        item.item_id,
                        item.name,
                        item.enabled as i64,
                        item.order,
                    ])?;
                }
            }

            /*
             * Anything the cloud no longer lists has been removed from the
             * sandbox elsewhere. Its staging folder is left on disk — deleting
             * downloaded files as a side effect of a list sync is not something
             * a user asked for, and the next deploy takes its files back out of
             * the game folder regardless.
             */
            let placeholders = if keys.is_empty() {
                "''".to_string()
            } else {
                keys.iter().map(|_| "?").collect::<Vec<_>>().join(",")
            };

            let sql = format!(
                "DELETE FROM sandbox_mod WHERE sandbox_id = ? AND mod_key NOT IN ({placeholders})"
            );

            let mut bound: Vec<&dyn rusqlite::ToSql> = vec![&id];

            for key in &keys {
                bound.push(key);
            }

            tx.execute(&sql, bound.as_slice())?;

            tx.commit()?;

            Ok(())
        })
    }

    /// Delete sandboxes whose cloud row is gone.
    ///
    /// Only ones that HAVE a `remote_id`. A local-only sandbox is not "missing
    /// from the server", it was never sent, and a sync that deleted those would
    /// make the privacy switch a data-loss switch.
    pub fn sandbox_retain_remote(&self, keep: &[i64]) -> AppResult<usize> {
        self.with(|conn| {
            let list = keep
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",");

            let sql = if keep.is_empty() {
                "DELETE FROM sandbox WHERE remote_id IS NOT NULL".to_string()
            } else {
                format!(
                    "DELETE FROM sandbox WHERE remote_id IS NOT NULL AND remote_id NOT IN ({list})"
                )
            };

            conn.execute(&sql, [])
        })
    }
}

/// One cloud install, ready to be mirrored.
pub struct RemoteSandbox<'a> {
    pub remote_id: i64,
    pub app_id: i64,
    pub app_slug: Option<&'a str>,
    pub app_name: Option<&'a str>,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub environment: Environment,
    pub strategy: Strategy,
    pub game_version: Option<&'a str>,
    pub loader: Option<&'a str>,
    pub is_default: bool,
    /// JSON, verbatim from the payload.
    pub options: String,
    pub launch_args: String,
    pub launch_env: String,
    pub items: &'a [RemoteItem],
}

pub struct RemoteItem {
    pub kind: String,
    pub item_id: i64,
    pub name: String,
    pub enabled: bool,
    pub order: i64,
}

impl RemoteItem {
    pub fn mod_key(&self) -> String {
        SandboxMod::key_for(&self.kind, self.item_id)
    }
}

const SANDBOX_SELECT: &str = "SELECT
    id, remote_id, app_id, app_slug, app_name, name, description,
    environment, strategy, game_version, loader, preset, is_default,
    cloud_sync, game_dir, options, launch_args, launch_env,
    deployed_at, last_deploy, created_at, updated_at, auto_update
 FROM sandbox";

fn row_to_sandbox(row: &rusqlite::Row<'_>) -> rusqlite::Result<Sandbox> {
    let options: String = row.get(15)?;
    let launch_args: String = row.get(16)?;
    let launch_env: String = row.get(17)?;
    let last_deploy: Option<String> = row.get(19)?;

    Ok(Sandbox {
        id: row.get(0)?,
        remote_id: row.get(1)?,
        app_id: row.get(2)?,
        app_slug: row.get(3)?,
        app_name: row.get(4)?,
        name: row.get(5)?,
        description: row.get(6)?,
        // A stored value we do not recognise degrades to the default rather
        // than failing the read: the database outlives any one app version,
        // and a sandbox that will not load is worse than one that loads as a
        // client sandbox.
        environment: Environment::parse(&row.get::<_, String>(7)?).unwrap_or_default(),
        strategy: Strategy::parse(&row.get::<_, String>(8)?).unwrap_or_default(),
        game_version: row.get(9)?,
        loader: row.get(10)?,
        preset: row.get(11)?,
        is_default: row.get::<_, i64>(12)? != 0,
        cloud_sync: row.get::<_, i64>(13)? != 0,
        game_dir: row.get(14)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
        launch_args: serde_json::from_str(&launch_args).unwrap_or_default(),
        launch_env: serde_json::from_str(&launch_env).unwrap_or_default(),
        deployed_at: row.get(18)?,
        last_deploy: last_deploy.and_then(|raw| serde_json::from_str(&raw).ok()),
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
        auto_update: row.get::<_, i64>(22)? != 0,
        mods: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> LibraryDb {
        LibraryDb::open_memory().expect("db")
    }

    fn new_sandbox(name: &str) -> NewSandbox {
        NewSandbox {
            app_id: 1,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: name.into(),
            description: None,
            environment: Environment::Client,
            strategy: Strategy::Hardlink,
            game_version: Some("1.21".into()),
            loader: Some("fabric".into()),
            preset: Some("fabric-1.21".into()),
            game_dir: None,
            options: BTreeMap::from([("memoryMb".into(), serde_json::json!(4096))]),
            cloud_sync: true,
            auto_update: true,
        }
    }

    #[test]
    fn a_sandbox_round_trips_with_its_options() {
        let db = db();

        let id = db
            .sandbox_create(&new_sandbox("Kitchen sink"))
            .expect("create");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert_eq!(found.name, "Kitchen sink");
        assert_eq!(found.environment, Environment::Client);
        assert_eq!(found.strategy, Strategy::Hardlink);
        assert_eq!(found.preset.as_deref(), Some("fabric-1.21"));
        assert_eq!(
            found.options.get("memoryMb"),
            Some(&serde_json::json!(4096))
        );
        assert!(found.is_default, "the first sandbox for a game is default");
        assert!(found.remote_id.is_none());
    }

    #[test]
    fn only_one_sandbox_per_game_is_the_default() {
        let db = db();

        let first = db.sandbox_create(&new_sandbox("One")).expect("create");
        let second = db.sandbox_create(&new_sandbox("Two")).expect("create");

        assert_eq!(db.sandbox_default_for(1).expect("default"), Some(first));

        db.sandbox_set_default(second).expect("set default");

        assert_eq!(db.sandbox_default_for(1).expect("default"), Some(second));
        assert!(!db.sandbox_get(first).unwrap().unwrap().is_default);
    }

    #[test]
    fn a_patch_leaves_untouched_fields_alone() {
        let db = db();
        let id = db.sandbox_create(&new_sandbox("One")).expect("create");

        db.sandbox_patch(
            id,
            &SandboxPatch {
                name: Some("Renamed".into()),
                ..Default::default()
            },
        )
        .expect("patch");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert_eq!(found.name, "Renamed");
        // Everything else survived.
        assert_eq!(found.loader.as_deref(), Some("fabric"));
        assert_eq!(
            found.options.get("memoryMb"),
            Some(&serde_json::json!(4096))
        );
    }

    /// `null` and "absent" are different requests, and conflating them is how a
    /// rename silently clears somebody's game version.
    #[test]
    fn a_patch_can_clear_a_field_without_clearing_its_neighbours() {
        let db = db();
        let id = db.sandbox_create(&new_sandbox("One")).expect("create");

        db.sandbox_patch(
            id,
            &SandboxPatch {
                loader: Some(None),
                ..Default::default()
            },
        )
        .expect("patch");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert_eq!(found.loader, None);
        assert_eq!(found.game_version.as_deref(), Some("1.21"));
    }

    #[test]
    fn mods_keep_their_load_order_and_can_be_reordered() {
        let db = db();
        let id = db.sandbox_create(&new_sandbox("One")).expect("create");

        let a = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");
        let b = db.sandbox_add_mod(id, "mod", 2, "Beta").expect("add");
        let c = db.sandbox_add_mod(id, "asset", 3, "Gamma").expect("add");

        let mods = db.sandbox_mods(id).expect("mods");

        assert_eq!(
            mods.iter().map(|m| m.mod_key.as_str()).collect::<Vec<_>>(),
            vec![a.as_str(), b.as_str(), c.as_str()],
            "added order is the initial load order"
        );

        db.sandbox_reorder(id, &[c.clone(), a.clone()])
            .expect("reorder");

        let mods = db.sandbox_mods(id).expect("mods");

        assert_eq!(
            mods.iter().map(|m| m.mod_key.as_str()).collect::<Vec<_>>(),
            vec![c.as_str(), a.as_str(), b.as_str()],
            "unnamed entries keep their place after the named ones"
        );
    }

    #[test]
    fn disabling_a_mod_keeps_it_in_the_list() {
        let db = db();
        let id = db.sandbox_create(&new_sandbox("One")).expect("create");

        let key = db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");

        db.sandbox_set_mod_enabled(id, &key, false)
            .expect("disable");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert_eq!(found.mods.len(), 1);
        assert_eq!(found.enabled_mods().count(), 0);
    }

    #[test]
    fn a_ledger_round_trips_and_replaces_wholesale() {
        let db = db();
        let id = db.sandbox_create(&new_sandbox("One")).expect("create");

        let entry = |path: &str| LedgerEntry {
            path: path.into(),
            kind: "hard".into(),
            mod_key: "mod:1".into(),
            source: "/staging/a.jar".into(),
            size: 10,
            mtime_ms: 1,
            backup: None,
        };

        db.sandbox_set_ledger(id, &[entry("mods/a.jar"), entry("mods/b.jar")])
            .expect("write");

        assert_eq!(db.sandbox_ledger(id).expect("read").len(), 2);

        db.sandbox_set_ledger(id, &[entry("mods/a.jar")])
            .expect("write");

        let rows = db.sandbox_ledger(id).expect("read");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, "mods/a.jar");
    }

    #[test]
    fn deleting_a_sandbox_takes_its_mods_and_ledger_with_it() {
        let db = db();
        let id = db.sandbox_create(&new_sandbox("One")).expect("create");

        db.sandbox_add_mod(id, "mod", 1, "Alpha").expect("add");
        db.sandbox_set_ledger(
            id,
            &[LedgerEntry {
                path: "a".into(),
                kind: "copy".into(),
                mod_key: "mod:1".into(),
                source: String::new(),
                size: 0,
                mtime_ms: 0,
                backup: None,
            }],
        )
        .expect("ledger");

        db.sandbox_delete(id).expect("delete");

        assert!(db.sandbox_get(id).expect("get").is_none());
        assert!(db.sandbox_mods(id).expect("mods").is_empty());
        assert!(db.sandbox_ledger(id).expect("ledger").is_empty());
    }

    // -------------------------------------------------------- Cloud mirroring

    fn remote(items: &[RemoteItem]) -> RemoteSandbox<'_> {
        RemoteSandbox {
            remote_id: 77,
            app_id: 1,
            app_slug: Some("minecraft"),
            app_name: Some("Minecraft"),
            name: "From the cloud",
            description: None,
            environment: Environment::Client,
            strategy: Strategy::Symlink,
            game_version: Some("1.21"),
            loader: Some("fabric"),
            is_default: false,
            options: "{}".into(),
            launch_args: "[]".into(),
            launch_env: "{}".into(),
            items,
        }
    }

    #[test]
    fn a_cloud_install_becomes_a_sandbox_and_updates_in_place() {
        let db = db();

        let items = vec![RemoteItem {
            kind: "mod".into(),
            item_id: 1,
            name: "Alpha".into(),
            enabled: true,
            order: 0,
        }];

        let id = db.sandbox_upsert_remote(&remote(&items)).expect("upsert");
        let again = db.sandbox_upsert_remote(&remote(&items)).expect("upsert");

        assert_eq!(id, again, "the same cloud row is the same sandbox");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert_eq!(found.remote_id, Some(77));
        assert_eq!(found.strategy, Strategy::Symlink);
        assert_eq!(found.mods.len(), 1);
    }

    /// The regression that costs somebody a 12 GB re-download: a sync must not
    /// forget which release is already staged.
    #[test]
    fn a_cloud_sync_does_not_forget_what_is_staged() {
        let db = db();

        let items = vec![RemoteItem {
            kind: "mod".into(),
            item_id: 1,
            name: "Alpha".into(),
            enabled: true,
            order: 0,
        }];

        let id = db.sandbox_upsert_remote(&remote(&items)).expect("upsert");

        db.sandbox_mark_staged(id, "mod:1", Some(9), Some("1.4"))
            .expect("staged");

        db.sandbox_upsert_remote(&remote(&items)).expect("upsert");

        let found = db.sandbox_get(id).expect("get").expect("present");

        assert_eq!(found.mods[0].release_id, Some(9));
        assert_eq!(found.mods[0].version.as_deref(), Some("1.4"));
        assert!(found.mods[0].staged_at.is_some());
    }

    #[test]
    fn an_item_dropped_in_the_cloud_leaves_the_sandbox() {
        let db = db();

        let both = vec![
            RemoteItem {
                kind: "mod".into(),
                item_id: 1,
                name: "Alpha".into(),
                enabled: true,
                order: 0,
            },
            RemoteItem {
                kind: "mod".into(),
                item_id: 2,
                name: "Beta".into(),
                enabled: true,
                order: 1,
            },
        ];

        let id = db.sandbox_upsert_remote(&remote(&both)).expect("upsert");

        assert_eq!(db.sandbox_mods(id).expect("mods").len(), 2);

        db.sandbox_upsert_remote(&remote(&both[..1]))
            .expect("upsert");

        let mods = db.sandbox_mods(id).expect("mods");

        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].mod_key, "mod:1");
    }

    /// The privacy switch has to actually switch something off. A sandbox the
    /// user took off the cloud must stop taking the server's word for anything.
    #[test]
    fn a_sandbox_taken_off_the_cloud_stops_being_overwritten_by_it() {
        let db = db();

        let items: Vec<RemoteItem> = vec![];
        let id = db.sandbox_upsert_remote(&remote(&items)).expect("upsert");

        db.sandbox_patch(
            id,
            &SandboxPatch {
                cloud_sync: Some(false),
                name: Some("Mine alone".into()),
                ..Default::default()
            },
        )
        .expect("patch");

        db.sandbox_upsert_remote(&remote(&items)).expect("upsert");

        assert_eq!(
            db.sandbox_get(id).unwrap().unwrap().name,
            "Mine alone",
            "the server must not rename a sandbox that opted out"
        );
    }

    #[test]
    fn a_local_only_sandbox_survives_a_cloud_reconcile() {
        let db = db();

        let mut local = new_sandbox("Local only");
        local.cloud_sync = false;

        let local_id = db.sandbox_create(&local).expect("create");

        let items: Vec<RemoteItem> = vec![];
        let remote_id = db.sandbox_upsert_remote(&remote(&items)).expect("upsert");

        // The server now lists nothing at all.
        db.sandbox_retain_remote(&[]).expect("retain");

        assert!(
            db.sandbox_get(local_id).expect("get").is_some(),
            "a sandbox that was never sent cannot be missing from the server"
        );
        assert!(db.sandbox_get(remote_id).expect("get").is_none());
    }

    #[test]
    fn environments_accept_shared_in_both_directions() {
        use Environment::*;

        assert!(Client.accepts(Client));
        assert!(Client.accepts(Shared));
        assert!(Shared.accepts(Server));
        assert!(!Client.accepts(Server));
        assert!(!Server.accepts(Client));
    }
}
