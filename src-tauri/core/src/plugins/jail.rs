use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult};
use crate::plugins::manifest::{FsRoot, Manifest, PathRef};

/// The path jail.
///
/// Every filesystem operation an installer plugin performs goes through
/// [`Jail::resolve`], and there is no other way to obtain a `PathBuf` in the
/// executor. What that buys is a single place to be right about a problem that
/// is famously easy to get wrong.
///
/// Three separate things are enforced, because any one of them alone has a
/// known bypass:
///
///   1. **Lexical rejection.** No `..`, no absolute path, no drive prefix, no
///      UNC root, no NUL. This catches the obvious `../../.ssh/authorized_keys`
///      before any syscall runs.
///   2. **Post-join containment.** The joined path is normalised and re-checked
///      against the root. This catches component sequences that are individually
///      fine but combine badly.
///   3. **Canonical containment of the deepest existing ancestor.** Symlinks are
///      resolved and the result must still be inside the root. This is the one
///      that catches an *existing* `mods/x -> /etc` planted by an earlier step,
///      or by anything else on the machine — a check on the literal path cannot
///      see it, and a check after opening the file is already too late.
///
/// Step 3 walks up to the deepest ancestor that exists because the target
/// itself usually does not yet: canonicalising a path that is about to be
/// created fails on every platform, so the guarantee has to come from its
/// parent chain instead.
pub struct Jail {
    roots: HashMap<&'static str, RootGrant>,
}

/// One root the plugin may reach, and the subtrees of it that it may touch.
///
/// `base` is always the ROOT — never a grant's subdirectory — and a `PathRef`'s
/// path is resolved against it. The grants are then *filters* over the result.
///
/// The alternative, anchoring each grant at its own subdirectory, cannot
/// represent two grants for the same root: a `PathRef` names only the root, so
/// there is no way to say which grant it meant. Merging them, which is what
/// this used to do, let a manifest declaring `{gameDir, ""}` read-only plus
/// `{gameDir, "mods"}` writable end up with the whole game directory writable —
/// wider than the consent dialog had displayed.
struct RootGrant {
    base: PathBuf,
    allowed: Vec<Allowed>,
}

struct Allowed {
    /// Absolute, under `base`. Equal to `base` for a whole-root grant.
    prefix: PathBuf,
    write: bool,
}

fn root_key(root: FsRoot) -> &'static str {
    match root {
        FsRoot::GameDir => "gameDir",
        FsRoot::PluginData => "pluginData",
        FsRoot::Downloads => "downloads",
    }
}

impl Jail {
    /// Build the jail from the manifest's grants and the app's resolved paths.
    ///
    /// `available` maps a root name to where it actually lives on THIS machine
    /// — resolved from user settings and the app's data dir, never from the
    /// manifest. A grant for a root the user has not configured is dropped, so
    /// the plugin fails with "no game directory set" rather than writing
    /// somewhere plausible-looking.
    pub fn build(
        manifest: &Manifest,
        available: &HashMap<&'static str, PathBuf>,
    ) -> AppResult<Self> {
        let mut roots: HashMap<&'static str, RootGrant> = HashMap::new();

        for grant in &manifest.permissions.fs {
            let key = root_key(grant.root);

            let Some(root) = available.get(key) else {
                continue;
            };

            std::fs::create_dir_all(root).map_err(|e| {
                AppError::jail(format!("Could not prepare {}: {e}", root.display()))
            })?;

            /*
             * Canonicalise the ROOT once, here. Everything downstream is
             * compared against this value, so if the root is itself reached
             * through a symlink (a very common macOS `/var` → `/private/var`
             * situation) the comparison still works.
             */
            let base = root.canonicalize().unwrap_or_else(|_| root.clone());

            // The subdirectory in the grant is itself untrusted text.
            let prefix = if grant.path.is_empty() {
                base.clone()
            } else {
                join_relative(&base, &grant.path)?
            };

            if !prefix.starts_with(&base) {
                return Err(AppError::jail(format!(
                    "'{}' escapes its root.",
                    grant.path
                )));
            }

            // Create the granted subtree so the first write into it does not
            // have to. Best effort: a read-only grant for a directory that does
            // not exist yet is not an error until something reads from it.
            if grant.write {
                let _ = std::fs::create_dir_all(&prefix);
            }

            let entry = roots.entry(key).or_insert_with(|| RootGrant {
                base: base.clone(),
                allowed: Vec::new(),
            });

            entry.allowed.push(Allowed {
                prefix,
                write: grant.write,
            });
        }

        Ok(Self { roots })
    }

    /// Resolve a manifest path to a real one, or refuse.
    ///
    /// `path_ref.path` is relative to the ROOT, not to any one grant — so
    /// `{ root: gameDir, path: "mods/foo.jar" }` is the whole story, and a
    /// plugin holding several narrow grants under one root needs no way to say
    /// which of them it meant.
    pub fn resolve(&self, path_ref: &PathRef, write: bool) -> AppResult<PathBuf> {
        let key = root_key(path_ref.root);

        let grant = self.roots.get(key).ok_or_else(|| {
            AppError::jail(format!(
                "'{key}' is not available — the plugin did not request it, or it is not configured."
            ))
        })?;

        let joined = join_relative(&grant.base, &path_ref.path)?;

        // (2) Containment after lexical normalisation.
        if !joined.starts_with(&grant.base) {
            return Err(AppError::jail(format!(
                "'{}' escapes its root.",
                path_ref.path
            )));
        }

        /*
         * (2b) The path must fall inside a subtree the manifest declared, with
         * the access it declared. Read and write are checked separately: a
         * grant that only permits reads must not satisfy a write, even for a
         * path it does cover.
         */
        let covered = grant
            .allowed
            .iter()
            .any(|a| joined.starts_with(&a.prefix) && (!write || a.write));

        if !covered {
            let readable = grant.allowed.iter().any(|a| joined.starts_with(&a.prefix));

            return Err(AppError::jail(if readable {
                format!("'{}' was granted read-only.", path_ref.path)
            } else {
                format!(
                    "'{}' is outside the folders this plugin asked for.",
                    path_ref.path
                )
            }));
        }

        // (3) Containment after symlink resolution of the deepest existing
        // ancestor.
        assert_real_ancestor_inside(&joined, &grant.base)?;

        Ok(joined)
    }

    pub fn root_path(&self, root: FsRoot) -> Option<&Path> {
        self.roots.get(root_key(root)).map(|g| g.base.as_path())
    }

    /// Is an ALREADY-RESOLVED absolute path inside a writable subtree of this
    /// jail?
    ///
    /// The narrow companion to [`Self::resolve`], and the only other way a path
    /// is admitted. `resolve` takes a manifest-supplied *relative* path and
    /// builds the real one; this takes a real one back — specifically, one the
    /// executor recorded in its journal on a previous run — and asks whether it
    /// is still ours to delete.
    ///
    /// It exists because that journal lives in a user-writable database on
    /// disk. It is our own record and it is still not trusted: without this
    /// check, an edited row would turn the uninstall path into a
    /// delete-anything primitive, which is precisely what the jail is for.
    ///
    /// Write access is required, not optional. Nothing calls this to READ.
    pub fn contains(&self, path: &Path) -> bool {
        if !path.is_absolute() {
            return false;
        }

        // Refuse a path carrying `..` or `.` outright rather than normalising
        // it: `a/../../b` can be lexically inside a base and really outside it,
        // and a journal entry should never have contained one in the first
        // place — the executor writes fully-resolved paths.
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        }) {
            return false;
        }

        self.roots.values().any(|grant| {
            path.starts_with(&grant.base)
                && grant
                    .allowed
                    .iter()
                    .any(|a| a.write && path.starts_with(&a.prefix))
                // The same symlink check `resolve` ends with: a link planted
                // since the install must not redirect the delete out of the jail.
                && assert_real_ancestor_inside(path, &grant.base).is_ok()
        })
    }
}

/// Join a *relative* path onto a base, rejecting everything that is not one.
///
/// Deliberately not `Path::join`: `base.join("/etc/passwd")` returns
/// `/etc/passwd`, discarding the base entirely, and `base.join("C:\\x")` does
/// the same on Windows. That single behaviour is the reason most path-jail bugs
/// exist, so this never calls it with untrusted input.
pub fn join_relative(base: &Path, relative: &str) -> AppResult<PathBuf> {
    if relative.contains('\0') {
        return Err(AppError::jail("Path contains a NUL byte."));
    }

    if relative.len() > 1024 {
        return Err(AppError::jail("Path is too long."));
    }

    // Accept `/` from manifests on every platform; normalise to the native
    // separator by walking components ourselves.
    let normalised = relative.replace('\\', "/");
    let candidate = Path::new(&normalised);

    let mut out = base.to_path_buf();

    for component in candidate.components() {
        match component {
            Component::Normal(part) => {
                let text = part.to_string_lossy();

                /*
                 * Windows silently strips trailing dots and spaces, so
                 * `evil.` and `evil` name the same file — a check that passes
                 * on the first can be a write to the second. Refusing both is
                 * simpler than modelling the platform's rules.
                 */
                if text.ends_with('.') || text.ends_with(' ') {
                    return Err(AppError::jail(
                        "Path components may not end with a dot or space.",
                    ));
                }

                // Windows alternate data streams: `file.txt:hidden`.
                if text.contains(':') {
                    return Err(AppError::jail("Path components may not contain ':'."));
                }

                out.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir => return Err(AppError::jail("Path may not contain '..'.")),
            Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::jail("Path must be relative."))
            }
        }
    }

    Ok(out)
}

/// Walk up to the deepest existing ancestor, canonicalise it, and require the
/// result to still be under `root`.
fn assert_real_ancestor_inside(target: &Path, root: &Path) -> AppResult<()> {
    let mut cursor = target;

    loop {
        if cursor.exists() {
            let real = cursor.canonicalize().map_err(|e| {
                AppError::jail(format!("Could not resolve {}: {e}", cursor.display()))
            })?;

            if !real.starts_with(root) {
                return Err(AppError::jail(format!(
                    "'{}' resolves outside its root (symlink?).",
                    target.display()
                )));
            }

            return Ok(());
        }

        match cursor.parent() {
            // Reached the filesystem root without finding anything that exists.
            // The lexical check already proved the path is under `root`, so
            // there is nothing further to verify.
            Some(parent) if parent != cursor => cursor = parent,
            _ => return Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn traversal_is_refused() {
        let dir = base();

        for bad in [
            "../escape",
            "a/../../escape",
            "/etc/passwd",
            "..",
            "a/b/../../../x",
        ] {
            assert!(
                join_relative(dir.path(), bad).is_err(),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn windows_shapes_are_refused() {
        let dir = base();

        for bad in [
            "C:/x",
            "\\\\server\\share",
            "file.txt:stream",
            "name.",
            "name ",
        ] {
            assert!(
                join_relative(dir.path(), bad).is_err(),
                "{bad} should be refused"
            );
        }
    }

    #[test]
    fn ordinary_relative_paths_join() {
        let dir = base();

        let out = join_relative(dir.path(), "mods/cool.jar").expect("should join");
        assert!(out.starts_with(dir.path()));
        assert!(out.ends_with("cool.jar"));
    }

    // ------------------------------------------------------- Grant scoping

    use crate::plugins::manifest::{FsGrant, Manifest, Permissions};

    fn jail_with(grants: Vec<FsGrant>, root: &Path) -> Jail {
        let manifest = Manifest {
            manifest_version: 1,
            id: "com.test.plugin".into(),
            name: "T".into(),
            version: "1".into(),
            author: "T".into(),
            description: None,
            homepage: None,
            apps: vec![],
            permissions: Permissions {
                fs: grants,
                ..Default::default()
            },
            installer: None,
            server_query: None,
            theme: None,
            manager: None,
        };

        let mut available: HashMap<&'static str, PathBuf> = HashMap::new();
        available.insert("gameDir", root.to_path_buf());

        Jail::build(&manifest, &available).expect("builds")
    }

    fn grant(path: &str, write: bool) -> FsGrant {
        FsGrant {
            root: FsRoot::GameDir,
            path: path.into(),
            write,
        }
    }

    fn path_ref(path: &str) -> PathRef {
        PathRef {
            root: FsRoot::GameDir,
            path: path.into(),
        }
    }

    #[test]
    fn a_narrow_write_grant_does_not_widen_a_broad_read_one() {
        // The regression: declaring the whole root readable and one subfolder
        // writable used to merge into "the whole root is writable", which is
        // wider than the consent dialog displayed.
        let dir = base();

        let jail = jail_with(vec![grant("", false), grant("mods", true)], dir.path());

        assert!(jail.resolve(&path_ref("mods/a.jar"), true).is_ok());
        assert!(jail.resolve(&path_ref("config/x.cfg"), false).is_ok());

        // Writable ONLY under `mods`, whichever order the grants came in.
        assert!(jail.resolve(&path_ref("config/x.cfg"), true).is_err());
        assert!(jail.resolve(&path_ref("evil.exe"), true).is_err());
    }

    #[test]
    fn grant_order_does_not_change_what_is_allowed() {
        let dir = base();

        let reversed = jail_with(vec![grant("mods", true), grant("", false)], dir.path());

        assert!(reversed.resolve(&path_ref("mods/a.jar"), true).is_ok());
        assert!(reversed.resolve(&path_ref("config/x.cfg"), true).is_err());
    }

    #[test]
    fn several_narrow_grants_under_one_root_all_work() {
        let dir = base();

        let jail = jail_with(vec![grant("mods", true), grant("config", true)], dir.path());

        assert!(jail.resolve(&path_ref("mods/a.jar"), true).is_ok());
        assert!(jail.resolve(&path_ref("config/a.cfg"), true).is_ok());
        assert!(jail.resolve(&path_ref("saves/a.dat"), true).is_err());
    }

    #[test]
    fn a_path_outside_every_grant_is_refused_even_for_reads() {
        let dir = base();

        let jail = jail_with(vec![grant("mods", true)], dir.path());

        assert!(jail.resolve(&path_ref("saves/world.dat"), false).is_err());
    }

    #[test]
    fn a_sibling_directory_sharing_a_name_prefix_is_not_covered() {
        // `mods-backup` must not be matched by the `mods` grant. `Path::
        // starts_with` compares whole components, which is what makes this
        // hold — a string prefix check would let it through.
        let dir = base();

        let jail = jail_with(vec![grant("mods", true)], dir.path());

        assert!(jail.resolve(&path_ref("mods-backup/a.jar"), true).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_symlink_out_of_the_root_is_caught() {
        let root = base();
        let outside = base();

        let link = root.path().join("escape");
        std::os::unix::fs::symlink(outside.path(), &link).expect("symlink");

        let real_root = root.path().canonicalize().expect("canonicalize");

        // Lexically fine — `escape/file` has no `..` in it.
        let target = join_relative(&real_root, "escape/file").expect("joins");

        assert!(assert_real_ancestor_inside(&target, &real_root).is_err());
    }
}
