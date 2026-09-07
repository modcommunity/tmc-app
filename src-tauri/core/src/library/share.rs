//! **Sharing a sandbox**, as a string somebody can paste.
//!
//! Every mod manager studied for this has a version of it — r2modman's profile
//! codes, Gale's export — and every one exists for the same two moments:
//! "send me your modpack" and "I am setting up my second machine".
//!
//! WHY A CODE AND NOT A FILE
//! ------------------------
//! A file export needs a path to write to and a file import needs a path to
//! read from, and this app's whole architecture rests on the webview never
//! naming either. The one command that takes a path returns directory NAMES and
//! reads nothing; adding a file reader beside it to support a convenience
//! feature would be trading the model's central guarantee for a save dialog.
//!
//! A pasteable string needs none of that, and it is what the two managers above
//! actually use in practice for the same reason: the thing people do with an
//! exported profile is put it in a chat message.
//!
//! WHAT IS IN ONE, AND WHAT DELIBERATELY IS NOT
//! ------------------------------------------
//! **Item ids, never files.** A sandbox's mods are TMC items, so an id list
//! reconstructs it exactly and a code stays a few hundred bytes. Shipping the
//! files would make this a redistribution channel for other people's work, with
//! no download counted, no licence respected and no update path.
//!
//! **Nothing about this machine.** No game directory, no staging path, no
//! deployment ledger, no play time. All four are answers about one computer —
//! the same test the settings split uses — and a code is by definition going to
//! a different one. A game folder in a shared code is also a leak of somebody's
//! username and drive layout into a chat message.
//!
//! **A version, and a prefix.** [`CODE_PREFIX`] is what makes a wrong paste
//! fail with "that is not a sandbox code" rather than a JSON parse error, and
//! the version is what lets a later format be recognised as too new instead of
//! being half-read.

use std::collections::BTreeMap;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::deploy::Strategy;
use crate::error::{AppError, AppResult};
use crate::library::sandbox::{Environment, Sandbox};

/// Marks a string as one of ours, and says which format it is.
pub const CODE_PREFIX: &str = "TMC1-";

/// The format version inside the payload.
///
/// Separate from the prefix on purpose: the prefix identifies the ENCODING and
/// the version identifies the SHAPE, so a future change that keeps base64 and
/// adds a field bumps this alone and old codes keep working.
const FORMAT: u32 = 1;

/// Cap on how many items a code may carry.
///
/// A modpack of five hundred is real; a code claiming fifty thousand is a
/// paste that will hang the import in `subscribe`, one request at a time.
const MAX_MODS: usize = 500;

/// The most a code may be, decoded. Bounds a hostile paste before it is parsed.
const MAX_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedMod {
    pub kind: String,
    pub item_id: i64,
    /// The item's name AS EXPORTED. Display only — the importer re-reads it
    /// from the account's own subscription, because a name in a pasted string
    /// is a name somebody else chose.
    #[serde(default)]
    pub name: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i64,
}

fn yes() -> bool {
    true
}

/// A sandbox, as it travels.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedSandbox {
    pub v: u32,
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
    pub options: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub launch_args: Vec<String>,
    #[serde(default)]
    pub mods: Vec<SharedMod>,
}

/// Turn a sandbox into a code.
pub fn export(row: &Sandbox) -> AppResult<String> {
    let shared = SharedSandbox {
        v: FORMAT,
        app_id: row.app_id,
        app_slug: row.app_slug.clone(),
        app_name: row.app_name.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        environment: row.environment,
        strategy: row.strategy,
        game_version: row.game_version.clone(),
        loader: row.loader.clone(),
        preset: row.preset.clone(),
        options: row.options.clone(),
        launch_args: row.launch_args.clone(),
        /*
         * `game_dir`, `launch_env`, the ledger and every staging path are
         * absent, and that is the point rather than an omission — see the
         * module header. `launch_env` in particular routinely holds a machine's
         * own paths.
         */
        mods: row
            .mods
            .iter()
            .take(MAX_MODS)
            .map(|m| SharedMod {
                kind: m.kind.clone(),
                item_id: m.item_id,
                name: m.name.clone(),
                enabled: m.enabled,
                priority: m.priority,
            })
            .collect(),
    };

    let json = serde_json::to_vec(&shared)
        .map_err(|e| AppError::internal(format!("could not encode the sandbox: {e}")))?;

    Ok(format!(
        "{CODE_PREFIX}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    ))
}

/// Read a code back, refusing anything that is not one.
///
/// Every failure is a sentence somebody can act on. A code is pasted by hand,
/// so "it did not work" is the most common outcome and the message is the only
/// thing that tells them whether to re-copy it or update the app.
pub fn import(code: &str) -> AppResult<SharedSandbox> {
    let trimmed = code.trim();

    let Some(body) = trimmed.strip_prefix(CODE_PREFIX) else {
        return Err(AppError::invalid(
            "That does not look like a sandbox code. They start with TMC1-.",
        ));
    };

    /*
     * Bounded BEFORE the decode, which is what `MAX_BYTES` claims to do.
     *
     * Checking the decoded length meant a 64 MB pasted string had already been
     * allocated in full by the time it was rejected. Base64 is four characters
     * per three bytes, so the encoded ceiling is the plaintext ceiling times
     * 4/3 — rounded up, so a code exactly at the limit is not refused for being
     * one character over.
     */
    if body.len() > MAX_BYTES.div_ceil(3) * 4 {
        return Err(AppError::invalid("That code is too large to be a sandbox."));
    }

    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(body.as_bytes())
        .map_err(|_| {
            AppError::invalid(
                "That code is damaged — it was probably cut short when it was copied.",
            )
        })?;

    let mut shared: SharedSandbox = serde_json::from_slice(&raw)
        .map_err(|_| AppError::invalid("That code is not a sandbox this app understands."))?;

    if shared.v > FORMAT {
        return Err(AppError::invalid(
            "That code was made by a newer version of the app. Update, then try again.",
        ));
    }

    if shared.name.trim().is_empty() {
        shared.name = "Imported sandbox".into();
    }

    /*
     * De-duplicated BEFORE truncating, and the order matters.
     *
     * A duplicated item would be added twice and then fight itself in the merge
     * tree, for one path, forever. Truncating first meant a code padded with
     * five hundred copies of one mod kept the padding and discarded every
     * genuine entry after it — which is a working import of the wrong modpack.
     */
    let mut seen = std::collections::BTreeSet::new();

    shared
        .mods
        .retain(|m| seen.insert((m.kind.clone(), m.item_id)));

    // Truncated rather than refused: a code with six hundred mods is a real
    // modpack somebody wants, and importing five hundred of it with a count on
    // screen beats importing none of it.
    shared.mods.truncate(MAX_MODS);

    Ok(shared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> Sandbox {
        Sandbox {
            id: 1,
            remote_id: None,
            app_id: 42,
            app_slug: Some("minecraft".into()),
            app_name: Some("Minecraft".into()),
            name: "Modded".into(),
            description: Some("A pack".into()),
            environment: Environment::Client,
            strategy: Strategy::Hardlink,
            game_version: Some("1.20.1".into()),
            loader: Some("fabric".into()),
            preset: Some("fabric".into()),
            is_default: true,
            cloud_sync: true,
            auto_update: true,
            game_dir: Some("/home/somebody/games/minecraft".into()),
            options: BTreeMap::new(),
            launch_args: vec!["--demo".into()],
            launch_env: BTreeMap::new(),
            deployed_at: None,
            last_deploy: None,
            created_at: "2026-01-01".into(),
            updated_at: "2026-01-01".into(),
            mods: vec![crate::library::sandbox::SandboxMod {
                mod_key: "mod:7".into(),
                kind: "mod".into(),
                item_id: 7,
                name: "Sodium".into(),
                enabled: true,
                priority: 3,
                release_id: Some(11),
                version: Some("0.5".into()),
                staged_at: Some("2026-01-01".into()),
                last_error: None,
            }],
        }
    }

    #[test]
    fn a_sandbox_round_trips() {
        let code = export(&row()).expect("export");
        let back = import(&code).expect("import");

        assert_eq!(back.app_id, 42);
        assert_eq!(back.name, "Modded");
        assert_eq!(back.loader.as_deref(), Some("fabric"));
        assert_eq!(back.launch_args, vec!["--demo".to_string()]);
        assert_eq!(back.mods.len(), 1);
        assert_eq!(back.mods[0].item_id, 7);
        assert_eq!(back.mods[0].priority, 3);
    }

    /// The one thing a code must never carry.
    ///
    /// A game directory is an answer about ONE machine, it is a plugin jail
    /// anchor, and it contains somebody's username — and a code is by
    /// definition going somewhere else.
    #[test]
    fn a_code_carries_nothing_about_this_machine() {
        let code = export(&row()).expect("export");

        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(code.strip_prefix(CODE_PREFIX).expect("prefix"))
            .expect("decode");

        let text = String::from_utf8(raw).expect("utf8");

        assert!(!text.contains("/home/somebody"), "leaked a path: {text}");
        assert!(!text.contains("gameDir"));
        assert!(!text.contains("launchEnv"));
        // Nor anything about deployment, which describes files on one disk.
        assert!(!text.contains("deployedAt"));
        assert!(!text.contains("stagedAt"));
    }

    #[test]
    fn something_that_is_not_a_code_is_refused_by_name() {
        let err = import("hello there").expect_err("refused");

        assert!(err.to_string().contains("TMC1-"), "{err}");
    }

    #[test]
    fn a_truncated_code_says_so() {
        let code = export(&row()).expect("export");
        let cut = &code[..code.len() - 5];

        // Base64 that no longer decodes, or JSON that no longer parses. Either
        // way the user needs to re-copy it, and both messages say that.
        let err = import(cut).expect_err("refused");
        let text = err.to_string();

        assert!(
            text.contains("damaged") || text.contains("understands"),
            "{text}"
        );
    }

    #[test]
    fn a_newer_format_is_refused_rather_than_half_read() {
        let mut shared: SharedSandbox = import(&export(&row()).expect("export")).expect("import");

        shared.v = FORMAT + 1;

        let json = serde_json::to_vec(&shared).expect("encode");
        let code = format!(
            "{CODE_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
        );

        let err = import(&code).expect_err("refused");

        assert!(err.to_string().contains("newer version"), "{err}");
    }

    #[test]
    fn a_duplicated_item_is_kept_once() {
        let mut shared: SharedSandbox = import(&export(&row()).expect("export")).expect("import");

        shared.mods.push(shared.mods[0].clone());

        let json = serde_json::to_vec(&shared).expect("encode");
        let code = format!(
            "{CODE_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
        );

        assert_eq!(import(&code).expect("import").mods.len(), 1);
    }

    #[test]
    fn a_code_with_no_name_gets_one() {
        let mut source = row();

        source.name = "   ".into();

        let back = import(&export(&source).expect("export")).expect("import");

        assert_eq!(back.name, "Imported sandbox");
    }
}
