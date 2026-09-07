//! The whole chain, once, against a real filesystem.
//!
//! Every layer below has its own unit tests and they are all narrow by design:
//! the jail is tested against escape attempts, the merge tree against
//! priorities, the ledger against a file the user edited. What none of them
//! prove is that the layers still fit together — that a subscription becomes a
//! staged folder, that the staged folder becomes files the game can see, that
//! undeploying puts the folder back exactly as it was found.
//!
//! So this test does one pass of the real thing:
//!
//! ```text
//!   a stock game folder          ← files that must survive untouched
//!   a mod archive over HTTP      ← a real socket, a real zip
//!   an app plugin rule           ← the same manifest format a user would write
//!     ↓ stage                    ← the step executor, in the real jail
//!     ↓ deploy                   ← EVERY strategy this machine supports
//!     ↓ verify                   ← what the ledger claims, against the disk
//!     ↓ launch plan              ← resolved, never run
//!     ↓ purge
//!   the stock game folder again  ← byte for byte, and no extra files
//! ```
//!
//! WHAT IT DELIBERATELY DOES NOT DO
//! --------------------------------
//! **It does not run the game.** The launch plan is resolved and inspected;
//! spawning a process from a test suite is how a CI runner acquires a hung job.
//!
//! **The plugin does not download.** `Step::Download` refuses plain http and
//! that rule is worth more than this test — a loopback exemption would put a
//! plaintext hole in shipping code so that a test could run. The archive still
//! crosses a real socket: the test fetches it over loopback HTTP and drops it
//! where a download step would have, so what is skipped is `reqwest` inside the
//! executor rather than the transfer. The executor's own download path is
//! covered in `download::tests` against the same kind of server, including
//! resume, the ignored-range trap and checksum rejection.
//!
//! **It does not exercise USVFS.** That strategy publishes a blob instead of
//! placing files and is tested where it lives, in `deploy::engine`. Here it
//! would deploy nothing and there would be nothing to compare.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tmc_core::deploy::{self, Strategy};
use tmc_core::library::db::{LibraryDb, LibraryEntry};
use tmc_core::library::deploy::{
    deploy_sandbox, launch_plan, purge_sandbox, stage_mod, target_dir, verify_sandbox,
};
use tmc_core::library::sandbox::{NewSandbox, Sandbox};
use tmc_core::logging::Audit;
use tmc_core::plugins::apps::AppPlugins;
use tmc_core::plugins::JailRoots;
use tmc_core::settings::AppSettings;

// --------------------------------------------------------------------- Rules

/// The install rule, in the format a plugin author would actually write.
///
/// Two steps rather than one, because the interesting half is the second: the
/// extract writes into `gameDir`, which during staging is redirected to the
/// sandbox's staging folder. A rule cannot tell the difference, and that is the
/// property that lets every install rule written before sandboxes existed keep
/// working.
const MANAGE_RULE: &str = r#"{
    "manifestVersion": 1,
    "label": "Test mod (.zip)",
    "match": { "extensions": ["zip"] },
    "permissions": {
        "fs": [
            { "root": "downloads", "path": "", "write": true },
            { "root": "gameDir", "path": "", "write": true }
        ]
    },
    "manage": {
        "install": [
            {
                "action": "mkdir",
                "path": { "root": "gameDir", "path": "mods" }
            },
            {
                "action": "extract",
                "from": { "root": "downloads", "path": "{fileName}" },
                "to": { "root": "gameDir", "path": "" }
            }
        ],
        "uninstall": [
            { "action": "remove", "path": { "root": "gameDir", "path": "mods" } }
        ]
    }
}"#;

const LAUNCH_RULE: &str = r#"{
    "manifestVersion": 1,
    "label": "Test game",
    "permissions": {},
    "launch": {
        "exec": "game.sh",
        "execPlatform": { "windows": "game.exe" },
        "args": ["--profile", "{installName}"],
        "optionArgs": { "memoryMb": ["-Xmx{memoryMb}M"] }
    }
}"#;

// ------------------------------------------------------------------ Fixtures

/// Everything the run needs, on disk, in one temporary tree.
struct World {
    _tmp: tempfile::TempDir,
    db: LibraryDb,
    plugins: AppPlugins,
    roots: JailRoots,
    settings: AppSettings,
    audit: Arc<Audit>,
    http: reqwest::Client,
    staging: PathBuf,
    backups: PathBuf,
    /// The device's imported-mod store — shared across sandboxes, unlike
    /// `staging`. See `tmc_core::local`.
    local: PathBuf,
    game: PathBuf,
}

/// The files the game shipped with. Every one of them has to be here, with
/// exactly this content, when the test finishes.
const STOCK: [(&str, &str); 3] = [
    ("game.sh", "#!/bin/sh\necho stock\n"),
    ("config/settings.ini", "volume=7\n"),
    ("mods/README.txt", "put mods here\n"),
];

const APP_ID: i64 = 4242;
const SLUG: &str = "testgame";

fn world() -> World {
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path();

    let game = base.join("game");
    let data = base.join("data");
    let cache = base.join("cache");
    let staging = base.join("staging");
    let backups = base.join("backups");
    let local = base.join("local-mods");
    let downloads = base.join("downloads");
    let plugin_dir = base.join("plugins");

    for dir in [&game, &data, &cache, &staging, &backups, &downloads, &local] {
        std::fs::create_dir_all(dir).expect("mkdir");
    }

    for (rel, body) in STOCK {
        let path = game.join(rel);

        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    let rules = plugin_dir.join("app").join(SLUG);

    std::fs::create_dir_all(&rules).expect("mkdir");
    std::fs::write(rules.join("manage_mod.json"), MANAGE_RULE).expect("rule");
    std::fs::write(rules.join("launch.json"), LAUNCH_RULE).expect("rule");

    let plugins = AppPlugins::load(&plugin_dir);

    assert!(
        plugins.errors().is_empty(),
        "the test's own rules must parse: {:?}",
        plugins.errors()
    );

    let mut settings = AppSettings {
        download_dir: Some(downloads.display().to_string()),
        ..Default::default()
    };

    settings
        .game_dirs
        .insert(APP_ID.to_string(), game.display().to_string());

    World {
        db: LibraryDb::open(base.join("library.db")).expect("db"),
        plugins,
        roots: JailRoots { data, cache },
        settings,
        audit: Arc::new(Audit::new(base.join("audit.jsonl"))),
        http: reqwest::Client::new(),
        staging,
        backups,
        local,
        game,
        _tmp: tmp,
    }
}

impl World {
    fn ctx(&self) -> tmc_core::library::deploy::SandboxCtx<'_> {
        tmc_core::library::deploy::SandboxCtx {
            plugins: &self.plugins,
            roots: &self.roots,
            settings: &self.settings,
            http: &self.http,
            audit: &self.audit,
            downloads: None,
            staging_root: &self.staging,
            backup_root: &self.backups,
            local_root: &self.local,
        }
    }

    fn downloads_dir(&self) -> PathBuf {
        PathBuf::from(self.settings.download_dir.as_ref().expect("set"))
    }
}

/// A zip holding two mod files, built in memory.
fn archive() -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for (name, body) in [("mods/cool.jar", "COOL"), ("mods/extra/data.bin", "DATA")] {
        use std::io::Write;

        zip.start_file(name, options).expect("entry");
        zip.write_all(body.as_bytes()).expect("body");
    }

    zip.finish().expect("finish").into_inner()
}

/// Serve one body over loopback HTTP, once per connection.
///
/// Twenty lines rather than a dependency, and the same shape `download::tests`
/// uses — a real socket is the only way for the archive to arrive the way a
/// real one does.
async fn serve(body: Vec<u8>) -> u16 {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");

    let port = listener.local_addr().expect("addr").port();

    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };

            let mut buf = vec![0u8; 4096];
            let _ = socket.read(&mut buf).await;

            let mut response = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            )
            .into_bytes();

            response.extend_from_slice(&body);

            let _ = socket.write_all(&response).await;
            let _ = socket.flush().await;
        }
    });

    port
}

/// Every file under a directory, as (relative path, bytes).
fn snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();

    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();

            // `symlink_metadata`, so a symlink is recorded as a file rather
            // than followed into staging — a strategy that leaves symlinks
            // behind must fail this test, not quietly pass by reading through
            // them.
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };

            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            if meta.is_dir() {
                walk(&path, base, out);
            } else {
                out.insert(rel, std::fs::read(&path).unwrap_or_default());
            }
        }
    }

    walk(root, root, &mut out);

    out
}

fn subscribe(world: &World, port: u16) -> LibraryEntry {
    let entry = LibraryEntry {
        id: "mod:1".into(),
        kind: "mod".into(),
        item_id: 1,
        name: "Cool Mod".into(),
        description: None,
        image: None,
        web_url: "https://moddingcommunity.com/mods/1".into(),
        app_id: Some(APP_ID),
        app_name: Some("Test Game".into()),
        app_slug: Some(SLUG.into()),
        auto_update: true,
        notify_updates: true,
        paused: false,
        via_collection_id: None,
        installable: true,
        latest_release_id: Some(9),
        latest_version: Some("1.4".into()),
        file_url: Some(format!("http://127.0.0.1:{port}/cool.zip")),
        file_name: Some("cool.zip".into()),
        file_size: None,
        file_sha256: None,
        updated_at: "2026-01-01T00:00:00Z".into(),
        installed_release_id: None,
        installed_version: None,
        installed_install_id: None,
        installed_at: None,
        installed_files: vec![],
        state: "idle".into(),
        last_error: None,
    };

    world.db.upsert_remote(&entry).expect("subscribe");

    entry
}

fn sandbox_with(world: &World, strategy: Strategy) -> Sandbox {
    let id = world
        .db
        .sandbox_create(&NewSandbox {
            app_id: APP_ID,
            app_slug: Some(SLUG.into()),
            app_name: Some("Test Game".into()),
            name: format!("Profile {}", strategy.as_str()),
            description: None,
            environment: Default::default(),
            strategy,
            game_version: Some("1.21".into()),
            loader: None,
            preset: None,
            game_dir: None,
            options: BTreeMap::from([("memoryMb".into(), serde_json::json!(4096))]),
            cloud_sync: false,
            auto_update: true,
        })
        .expect("create");

    world
        .db
        .sandbox_add_mod(id, "mod", 1, "Cool Mod")
        .expect("add");

    world.db.sandbox_get(id).expect("get").expect("exists")
}

// ------------------------------------------------------------------ The test

/// One pass per strategy this machine can actually do.
///
/// Which those are is PROBED rather than assumed — a filesystem with no hard
/// links, a Windows box without Developer Mode and a CI container are all
/// legitimate places for this to run, and asserting that a machine supports
/// symlinks is how a test suite becomes environment-specific.
#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_becomes_a_deployed_game_folder_and_back_again() {
    let world = world();

    let port = serve(archive()).await;
    let entry = subscribe(&world, port);

    // The archive arrives over a real socket, and is put where a download step
    // would have left it — see the module header for why the step itself is not
    // what fetches it here.
    let bytes = reqwest::get(entry.file_url.clone().expect("url"))
        .await
        .expect("request")
        .bytes()
        .await
        .expect("body");

    std::fs::write(world.downloads_dir().join("cool.zip"), &bytes).expect("write");

    let stock = snapshot(&world.game);

    assert_eq!(
        stock.len(),
        STOCK.len(),
        "the fixture is what it says it is"
    );

    let available: Vec<Strategy> = deploy::available_strategies(&world.staging, &world.game)
        .into_iter()
        .filter(|s| s.available)
        .filter_map(|s| Strategy::parse(&s.strategy))
        // USVFS deploys no files, so there is nothing for this test to compare.
        .filter(|s| *s != Strategy::Usvfs)
        .collect();

    assert!(
        available.contains(&Strategy::Direct),
        "copying is always possible and is the fallback"
    );

    // Printed rather than asserted, so a run on a filesystem with no hard links
    // says so instead of silently covering less than it looks like it does.
    println!(
        "deploying by: {}",
        available
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    for strategy in available {
        let sandbox = sandbox_with(&world, strategy);
        let ctx = world.ctx();

        // ------------------------------------------------------------- Stage
        let member = sandbox.mods.first().expect("one mod").clone();
        let outcome = stage_mod(&world.db, &sandbox, &member, &entry, &ctx).await;

        assert!(
            outcome.ok,
            "{}: staging failed: {:?}",
            strategy.as_str(),
            outcome.error
        );

        let staged = deploy::stage_dir(&world.staging, sandbox.id, &member.mod_key);

        assert_eq!(
            std::fs::read_to_string(staged.join("mods/cool.jar")).expect("staged"),
            "COOL"
        );

        // Staging alone changes NOTHING in the game folder. The two halves are
        // separate on purpose and this is the assertion that keeps them so.
        assert_eq!(
            snapshot(&world.game),
            stock,
            "{}: staging touched the game folder",
            strategy.as_str()
        );

        // ------------------------------------------------------------ Deploy
        let sandbox = world.db.sandbox_get(sandbox.id).expect("get").expect("row");

        let planned =
            deploy_sandbox(&world.db, &sandbox, &ctx, true).expect("dry run must not fail");

        assert_eq!(planned.placed, 2, "{}: dry run", strategy.as_str());
        assert_eq!(
            snapshot(&world.game),
            stock,
            "{}: a dry run wrote something",
            strategy.as_str()
        );

        let report = deploy_sandbox(&world.db, &sandbox, &ctx, false).expect("deploy");

        assert!(report.ok(), "{}: {:?}", strategy.as_str(), report.errors);
        assert_eq!(report.placed, 2, "{}: placed", strategy.as_str());

        // The mod's files are readable THROUGH the game folder, whichever
        // mechanism put them there. That is the only thing a game cares about
        // and the only thing worth asserting across all three.
        assert_eq!(
            std::fs::read_to_string(world.game.join("mods/cool.jar")).expect("deployed"),
            "COOL"
        );
        assert_eq!(
            std::fs::read_to_string(world.game.join("mods/extra/data.bin")).expect("deployed"),
            "DATA"
        );

        // And the game's own files are still exactly what they were.
        for (rel, body) in STOCK {
            assert_eq!(
                std::fs::read_to_string(world.game.join(rel)).expect("stock file"),
                body,
                "{}: {rel} was disturbed",
                strategy.as_str()
            );
        }

        // ------------------------------------------------------------ Verify
        let checked = verify_sandbox(&world.db, &sandbox, &ctx).expect("verify");

        assert!(
            checked.healthy(),
            "{}: {:?} missing, {:?} changed",
            strategy.as_str(),
            checked.missing,
            checked.changed
        );
        assert_eq!(checked.intact, 2);

        // ------------------------------------------------------------ Launch
        //
        // Resolved and inspected, never run.
        let sandbox = world.db.sandbox_get(sandbox.id).expect("get").expect("row");
        let plan = launch_plan(&world.db, &sandbox, &ctx).expect("plan");

        let program = plan.program.clone().expect("a program, not a URI");

        assert!(
            program.starts_with(
                &target_dir(&sandbox, &world.settings)
                    .unwrap()
                    .display()
                    .to_string()
            ),
            "{}: the executable resolved outside the game folder: {program}",
            strategy.as_str()
        );
        assert!(plan.args.contains(&sandbox.name), "the profile name");
        assert!(
            plan.args.contains(&"-Xmx4096M".to_string()),
            "the sandbox's own option"
        );
        assert!(
            plan.vfs.is_none(),
            "{}: only a virtual deploy carries a tree",
            strategy.as_str()
        );

        // ------------------------------------------------------------- Purge
        let purged = purge_sandbox(&world.db, &sandbox, &ctx).expect("purge");

        assert_eq!(purged.removed, 2, "{}: removed", strategy.as_str());
        assert!(
            purged.errors.is_empty(),
            "{}: {:?}",
            strategy.as_str(),
            purged.errors
        );

        // The point of the whole exercise.
        assert_eq!(
            snapshot(&world.game),
            stock,
            "{}: the game folder did not come back to stock",
            strategy.as_str()
        );

        // Staging survives an undeploy, which is what makes switching between
        // sandboxes a link operation rather than a download.
        assert!(
            staged.join("mods/cool.jar").is_file(),
            "{}: purging deleted the staged copy",
            strategy.as_str()
        );

        world.db.sandbox_delete(sandbox.id).expect("delete");
    }
}

/// A file the game already had, at a path a mod also provides.
///
/// The one case where a deploy has to modify the game folder — and the one
/// where getting it wrong is unrecoverable, because the displaced file may be
/// the only copy in existence.
#[tokio::test(flavor = "multi_thread")]
async fn a_game_file_a_mod_replaces_is_set_aside_and_put_back() {
    let world = world();

    let port = serve(archive()).await;
    let entry = subscribe(&world, port);

    let bytes = reqwest::get(entry.file_url.clone().expect("url"))
        .await
        .expect("request")
        .bytes()
        .await
        .expect("body");

    std::fs::write(world.downloads_dir().join("cool.zip"), &bytes).expect("write");

    // The game ships its own `mods/cool.jar`, which the mod will replace.
    std::fs::write(world.game.join("mods/cool.jar"), "SHIPPED WITH THE GAME").expect("write");

    let stock = snapshot(&world.game);

    let sandbox = sandbox_with(&world, Strategy::Direct);
    let ctx = world.ctx();

    let member = sandbox.mods.first().expect("one mod").clone();

    assert!(
        stage_mod(&world.db, &sandbox, &member, &entry, &ctx)
            .await
            .ok
    );

    let sandbox = world.db.sandbox_get(sandbox.id).expect("get").expect("row");
    let report = deploy_sandbox(&world.db, &sandbox, &ctx, false).expect("deploy");

    assert!(report.ok(), "{:?}", report.errors);
    assert_eq!(report.backed_up, 1, "the shipped file was set aside");
    assert_eq!(
        std::fs::read_to_string(world.game.join("mods/cool.jar")).expect("read"),
        "COOL",
        "the mod won"
    );

    let purged = purge_sandbox(&world.db, &sandbox, &ctx).expect("purge");

    assert_eq!(purged.restored, 1);
    assert_eq!(
        snapshot(&world.game),
        stock,
        "the shipped file came back, byte for byte"
    );
}

/// An imported mod and a subscribed one, in one sandbox, through one deploy.
///
/// The claim `tmc_core::local` makes is that a mod the user dropped in is
/// deployed by exactly the code that deploys a subscribed one — same merge
/// tree, same conflict report, same ledger, same purge. A claim like that is
/// only worth anything if something checks it, and the cheap ways of checking
/// it (unit-testing `deployable`) prove the part that was never in doubt.
///
/// So: one sandbox holding both, sharing a contested path, deployed for real.
#[tokio::test(flavor = "multi_thread")]
async fn an_imported_mod_and_a_subscribed_one_deploy_through_one_ledger() {
    use tmc_core::local::{store, LocalMod, NewLocalMod, Origin};

    let world = world();

    let port = serve(archive()).await;
    let entry = subscribe(&world, port);

    let bytes = reqwest::get(entry.file_url.clone().expect("url"))
        .await
        .expect("request")
        .bytes()
        .await
        .expect("body");

    std::fs::write(world.downloads_dir().join("cool.zip"), &bytes).expect("write");

    let stock = snapshot(&world.game);

    let sandbox = sandbox_with(&world, Strategy::Direct);
    let ctx = world.ctx();

    // ------------------------------------------------------- The subscription
    let member = sandbox.mods.first().expect("one mod").clone();

    assert!(
        stage_mod(&world.db, &sandbox, &member, &entry, &ctx)
            .await
            .ok
    );

    // ---------------------------------------------------------- The import
    //
    // A zip somebody was handed, with no sidecar and no account behind it. It
    // provides one new file and one the subscribed mod also provides, so the
    // merge tree has something real to decide.
    let dropped = world
        .game
        .parent()
        .expect("parent")
        .join("handed-to-me.zip");

    std::fs::write(
        &dropped,
        zip_of(&[
            ("mods/handmade.jar", "HANDMADE"),
            ("mods/cool.jar", "IMPORTED VERSION"),
        ]),
    )
    .expect("write");

    let targets = vec!["mods".to_string()];

    let prepared = store::prepare(
        &world.local,
        &dropped,
        &store::ImportOptions {
            mod_targets: &targets,
            mod_extensions: &[],
            payload: None,
            rel_path: None,
        },
    )
    .expect("prepare");

    // The archive is already game-relative, so it is not nested inside `mods`
    // a second time.
    assert_eq!(prepared.rel_path, "");
    assert_eq!(prepared.name, "handed-to-me");

    let local_id = world
        .db
        .local_create(&NewLocalMod {
            name: prepared.name.clone(),
            app_id: Some(APP_ID),
            app_slug: Some(SLUG.into()),
            version: None,
            author: None,
            origin: Origin::Dropped,
            origin_label: Some("handed-to-me.zip".into()),
            rel_path: prepared.rel_path.clone(),
            source: None,
            files: prepared.files as i64,
            bytes: prepared.bytes as i64,
        })
        .expect("create");

    store::commit(&world.local, &prepared, local_id).expect("commit");

    let imported = world.db.local_get(local_id).expect("get").expect("row");

    world
        .db
        .sandbox_add_local(sandbox.id, &imported)
        .expect("add");

    // Importing writes to the app's own store and NOTHING else. The dropped
    // file is still where it was and the game folder has not been touched.
    assert!(dropped.is_file(), "the dropped file was consumed");
    assert_eq!(snapshot(&world.game), stock, "importing touched the game");

    // ------------------------------------------------------------- Deploy
    let sandbox = world.db.sandbox_get(sandbox.id).expect("get").expect("row");

    assert_eq!(sandbox.mods.len(), 2);

    let report = deploy_sandbox(&world.db, &sandbox, &ctx, false).expect("deploy");

    assert!(report.ok(), "{:?}", report.errors);

    // Three distinct paths across two mods, in one ledger.
    assert_eq!(report.placed, 3);

    // The import was added second, so it has the higher priority and wins the
    // contested path — by the ordinary rule, with no special case for being
    // local.
    assert_eq!(
        std::fs::read_to_string(world.game.join("mods/cool.jar")).expect("deployed"),
        "IMPORTED VERSION"
    );
    assert_eq!(
        std::fs::read_to_string(world.game.join("mods/handmade.jar")).expect("deployed"),
        "HANDMADE"
    );
    assert_eq!(
        std::fs::read_to_string(world.game.join("mods/extra/data.bin")).expect("deployed"),
        "DATA"
    );

    // And the conflict is REPORTED rather than silently resolved.
    let contested = report
        .conflicts
        .iter()
        .find(|c| c.path == "mods/cool.jar")
        .expect("the contested path is in the report");

    assert_eq!(contested.winner, imported.name);
    assert_eq!(contested.losers, vec!["Cool Mod".to_string()]);

    // ------------------------------------------------------------- Verify
    let checked = verify_sandbox(&world.db, &sandbox, &ctx).expect("verify");

    assert!(
        checked.healthy(),
        "{:?} missing, {:?} changed",
        checked.missing,
        checked.changed
    );

    // -------------------------------------------------------------- Purge
    let purged = purge_sandbox(&world.db, &sandbox, &ctx).expect("purge");

    assert_eq!(purged.removed, 3);
    assert_eq!(
        snapshot(&world.game),
        stock,
        "the game folder came back exactly as it was"
    );

    // Purging takes files OUT of the game. It does not touch the store — the
    // import is still on this device and still in the sandbox, which is what
    // makes redeploying it a click.
    assert!(store::local_root(&world.local, local_id)
        .join("mods/handmade.jar")
        .is_file());
    assert_eq!(
        LocalMod::id_from_key(&imported.mod_key()),
        Some(local_id),
        "the ledger and the store agree on the identity"
    );
}

/// A zip built in memory, for the import half of the test above.
fn zip_of(entries: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write as _;

    let mut buf = std::io::Cursor::new(Vec::new());

    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();

        for (name, body) in entries {
            writer.start_file(*name, options).expect("entry");
            writer.write_all(body.as_bytes()).expect("body");
        }

        writer.finish().expect("finish");
    }

    buf.into_inner()
}
