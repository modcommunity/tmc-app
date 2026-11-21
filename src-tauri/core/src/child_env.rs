//! The environment a launched game inherits from the app.
//!
//! Inside an AppImage the app runs under `AppRun`, which points the loader and
//! GTK at the bundle (`LD_LIBRARY_PATH`, `GTK_PATH`, `GDK_PIXBUF_MODULE_FILE`,
//! `GIO_EXTRA_MODULES`, `XDG_DATA_DIRS`, ...) and sets `APPDIR`/`APPIMAGE`. A
//! game started with those inherited loads the bundle's Mesa/libGL/libvulkan
//! instead of the host's, which on a rolling distribution (CachyOS) is a
//! different ABI from the driver: the window opens and stays gray. The game is
//! not part of the bundle and must see the host's environment.
//!
//! [`sanitize`] is pure (input pairs in, changes out) so it is unit-tested
//! without touching the process environment; [`apply`] feeds it the real one.
//! Off an AppImage (`APPDIR` unset) it changes nothing.

use std::process::Command;

/// One edit to the child's environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Remove(String),
    Set(String, String),
}

/// Variables that exist only because of the AppImage runtime.
const RUNTIME_VARS: &[&str] = &["APPDIR", "APPIMAGE", "ARGV0", "OWD"];

/// Work out what to change so a child does not see the AppImage's libraries.
pub fn sanitize(vars: &[(String, String)]) -> Vec<Change> {
    let get = |k: &str| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());

    let Some(appdir) = get("APPDIR")
        .map(|d| d.trim_end_matches('/'))
        .filter(|d| !d.is_empty())
    else {
        return Vec::new();
    };

    let mut out = Vec::new();

    for (name, value) in vars {
        if RUNTIME_VARS.contains(&name.as_str()) || name.starts_with("APPIMAGE_") {
            out.push(Change::Remove(name.clone()));

            continue;
        }

        if name.ends_with("_ORIG") || !value.contains(appdir) {
            continue;
        }

        // `*_ORIG` is what the variable was before AppRun touched it.
        if let Some(orig) = get(&format!("{name}_ORIG")) {
            out.push(if orig.is_empty() {
                Change::Remove(name.clone())
            } else {
                Change::Set(name.clone(), orig.to_string())
            });

            continue;
        }

        let parts: Vec<&str> = value.split(':').collect();
        let kept: Vec<&str> = parts
            .iter()
            .copied()
            .filter(|p| !p.is_empty() && !inside(p, appdir))
            .collect();

        if kept.is_empty() {
            out.push(Change::Remove(name.clone()));
        } else if kept.len() != parts.len() {
            out.push(Change::Set(name.clone(), kept.join(":")));
        }
    }

    out
}

fn inside(path: &str, appdir: &str) -> bool {
    path == appdir
        || path
            .strip_prefix(appdir)
            .is_some_and(|r| r.starts_with('/'))
}

/// Apply [`sanitize`] of the app's own environment to a command.
pub fn apply(command: &mut Command) {
    let vars: Vec<(String, String)> = std::env::vars().collect();

    for change in sanitize(&vars) {
        match change {
            Change::Remove(k) => {
                command.env_remove(k);
            }
            Change::Set(k, v) => {
                command.env(k, v);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn outside_an_appimage_nothing_changes() {
        assert!(sanitize(&env(&[("LD_LIBRARY_PATH", "/opt/x/lib")])).is_empty());
    }

    #[test]
    fn bundle_library_paths_are_stripped_and_host_ones_kept() {
        let c = sanitize(&env(&[
            ("APPDIR", "/tmp/.mount_abc"),
            (
                "LD_LIBRARY_PATH",
                "/tmp/.mount_abc/usr/lib:/opt/host/lib:/tmp/.mount_abc",
            ),
        ]));

        assert!(c.contains(&Change::Set("LD_LIBRARY_PATH".into(), "/opt/host/lib".into())));
    }

    #[test]
    fn a_variable_that_is_only_the_bundle_is_removed() {
        let c = sanitize(&env(&[
            ("APPDIR", "/tmp/.mount_abc"),
            ("GTK_PATH", "/tmp/.mount_abc/usr/lib/gtk-3.0"),
            ("GDK_PIXBUF_MODULE_FILE", "/tmp/.mount_abc/usr/lib/loaders.cache"),
        ]));

        assert!(c.contains(&Change::Remove("GTK_PATH".into())));
        assert!(c.contains(&Change::Remove("GDK_PIXBUF_MODULE_FILE".into())));
    }

    #[test]
    fn orig_values_are_restored() {
        let c = sanitize(&env(&[
            ("APPDIR", "/tmp/.mount_abc"),
            ("XDG_DATA_DIRS", "/tmp/.mount_abc/usr/share:/usr/share"),
            ("XDG_DATA_DIRS_ORIG", "/usr/local/share:/usr/share"),
        ]));

        assert!(c.contains(&Change::Set(
            "XDG_DATA_DIRS".into(),
            "/usr/local/share:/usr/share".into()
        )));
    }

    #[test]
    fn runtime_markers_go_and_a_lookalike_prefix_stays() {
        let c = sanitize(&env(&[
            ("APPDIR", "/tmp/.mount_abc"),
            ("APPIMAGE", "/home/u/app.AppImage"),
            ("PATH", "/tmp/.mount_abcd/bin:/usr/bin"),
        ]));

        assert!(c.contains(&Change::Remove("APPDIR".into())));
        assert!(c.contains(&Change::Remove("APPIMAGE".into())));
        // `/tmp/.mount_abcd` is a different directory from `/tmp/.mount_abc`.
        assert!(!c
            .iter()
            .any(|x| matches!(x, Change::Set(k, _) | Change::Remove(k) if k == "PATH")));
    }
}
