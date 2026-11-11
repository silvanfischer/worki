//! macOS ACL helpers. Access is granted per path, and everything next to the path from the
//! first folder others couldn't enter before is explicitly denied, so granting a path never
//! exposes its siblings.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

/// Full read/write access, inherited by everything created below a directory later.
pub const FULL_ACCESS: &str =
    "allow list,add_file,search,delete,add_subdirectory,delete_child,readattr,writeattr,\
readextattr,writeextattr,readsecurity,read,write,append,execute,file_inherit,directory_inherit";

/// Read and execute only, inherited the same way.
pub const READ_ONLY: &str =
    "allow list,search,readattr,readextattr,readsecurity,read,execute,file_inherit,directory_inherit";

/// Read and write for a single file, no inheritance.
pub const READ_WRITE_FILE: &str =
    "allow read,write,append,readattr,writeattr,readextattr,writeextattr,readsecurity";

/// Lets a path be traversed without listing it.
const ROUTE: &str = "allow search";

/// Marks a routed file the way [`ROUTE`] marks a routed folder.
const FILE_ROUTE: &str = "allow readattr";

/// Keeps a route folder from being listed even when its mode lets everyone read it.
const NO_LIST: &str = "deny list,add_file,add_subdirectory,delete_child";

const DENY: &str =
    "deny list,add_file,search,delete,add_subdirectory,delete_child,readattr,writeattr,\
readextattr,writeextattr,readsecurity,writesecurity,chown,read,write,append,execute";

/// Applied to route folders so entries created there later start out denied. `limit_inherit`
/// stops it at the direct children, so it never reaches into a granted worktree.
const DENY_NEW_ENTRIES: &str = "file_inherit,directory_inherit,only_inherit,limit_inherit";

/// Blocks rewriting a file (git metadata that redirects where git looks for config).
const DENY_MODIFY: &str = "deny write,append,delete,writeattr,writeextattr,writesecurity,chown";

/// The `chmod +a`/`-a` argument for one entry; with empty `rights`, the prefix of every
/// entry for `user` as `ls -le` prints it.
fn entry(user: &str, rights: &str) -> String {
    format!("user:{user} {rights}")
}

fn chmod(args: &[&str], paths: &[&Path]) -> Result<bool> {
    if paths.is_empty() {
        return Ok(true);
    }
    let status = Command::new("/bin/chmod")
        .args(args)
        .args(paths)
        .stderr(std::process::Stdio::null())
        .status()
        .context("chmod")?;
    Ok(status.success())
}

fn add(path: &Path, user: &str, ace: &str) -> Result<()> {
    if !chmod(&["-h", "+a", &entry(user, ace)], &[path])? {
        bail!("could not change permissions of {}", path.display());
    }
    Ok(())
}

/// The ACL lines of `path` (without the mode line).
fn entries(path: &Path) -> Result<Vec<String>> {
    let out = Command::new("/bin/ls")
        .arg("-led")
        .arg(path)
        .output()
        .context("ls -led")?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .map(|l| l.trim().to_owned())
        .collect())
}

fn has(path: &Path, user: &str, pred: impl Fn(&str) -> bool) -> Result<bool> {
    let needle = entry(user, "");
    Ok(entries(path)?
        .iter()
        .any(|l| l.contains(&needle) && pred(l)))
}

/// Whether `user` has an explicit full-access entry on `path`.
pub fn has_full(path: &Path, user: &str) -> Result<bool> {
    has(path, user, |l| {
        l.contains(" allow ") && l.contains("add_subdirectory") && !l.contains("inherited")
    })
}

fn has_allow(path: &Path, user: &str) -> Result<bool> {
    has(path, user, |l| l.contains(" allow "))
}

/// Whether an ACL line grants at least `access`. On directories macOS prints file rights under
/// their directory names (`read` as `list`, `write` as `add_file`, …), so those are skipped.
fn same_rights(line: &str, access: &str) -> bool {
    let (kind, rights) = access.split_once(' ').unwrap_or((access, ""));
    let listed: Vec<&str> = line
        .split_once(&format!(" {kind} "))
        .map(|(_, r)| r.split(',').collect())
        .unwrap_or_default();
    !listed.is_empty()
        && rights
            .split(',')
            .filter(|r| !matches!(*r, "read" | "write" | "append" | "execute"))
            .all(|r| listed.contains(&r))
}

/// Grants `user` `access` on `path` and everything below it. Skipped when the top already has
/// that explicit entry, since `chmod -R` over a large tree is slow.
pub fn grant_tree(path: &Path, user: &str, access: &str) -> Result<()> {
    if has(path, user, |l| {
        !l.contains("inherited") && same_rights(l, access)
    })? {
        return Ok(());
    }
    if !chmod(&["-R", "+a", &entry(user, access)], &[path])? {
        bail!("could not grant {user} access to {}", path.display());
    }
    Ok(())
}

pub fn grant_file(path: &Path, user: &str, access: &str) -> Result<()> {
    add(path, user, access)
}

pub fn deny_modify(path: &Path, user: &str) -> Result<()> {
    add(path, user, DENY_MODIFY)
}

/// Removes every entry for `user` with exactly `access` below `path`, including inherited copies.
pub fn revoke_tree(path: &Path, user: &str, access: &str) -> Result<()> {
    chmod(&["-R", "-a", &entry(user, access)], &[path])?;
    Ok(())
}

fn others_can_enter(path: &Path) -> Result<bool> {
    Ok(std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .mode()
        & 0o001
        != 0)
}

fn current_uid() -> u32 {
    Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(u32::MAX)
}

/// Makes each target reachable for `user` and nothing else.
///
/// From the first ancestor that others couldn't enter, every folder on the way gets a
/// traverse-only entry and an inherit-only deny for future entries; every entry next to the
/// route is denied unless it is itself on a route `user` was given earlier (it then carries an
/// allow entry for `user`).
///
/// When every ancestor is open to others (a 755 home, `/Users/Shared`, most external drives),
/// the same starts at the first ancestor you own, else at the target's parent, but only your
/// own folders and entries get entries: other people's can't be changed by you.
pub fn confine(targets: &[PathBuf], user: &str) -> Result<()> {
    confine_below(targets, user, Path::new("/"))
}

fn confine_below(targets: &[PathBuf], user: &str, floor: &Path) -> Result<()> {
    let uid = current_uid();
    let on_route = |p: &Path| targets.iter().any(|t| t.starts_with(p));

    for target in targets {
        let mut chain: Vec<&Path> = target
            .ancestors()
            .skip(1)
            .filter(|a| a.starts_with(floor))
            .collect();
        chain.reverse();
        if chain.is_empty() {
            continue;
        }
        let owned = |p: &Path| std::fs::symlink_metadata(p).is_ok_and(|m| m.uid() == uid);
        let mut private = None;
        for (i, dir) in chain.iter().enumerate() {
            if !others_can_enter(dir)? {
                private = Some(i);
                break;
            }
        }
        let open = private.is_none();
        let start = private
            .or_else(|| chain.iter().position(|d| owned(d)))
            .unwrap_or(chain.len() - 1);

        if !open {
            if let Some(dir) = chain[start..].iter().find(|d| !owned(d)) {
                bail!(
                    "{} is private and not owned by you, so it can't be opened for {user}",
                    dir.display()
                );
            }
        }
        let ours: Vec<&Path> = chain[start..]
            .iter()
            .copied()
            .filter(|d| !open || owned(d))
            .collect();
        // `chmod -a` strips these permissions from every deny entry, so it has to run before
        // the route's own deny entries are added.
        let route: Vec<&Path> = chain[start + 1..]
            .iter()
            .copied()
            .chain([target.as_path()])
            .filter(|p| !open || owned(p))
            .collect();
        chmod(&["-h", "-a", &entry(user, DENY)], &route)?;
        for dir in &ours {
            add(dir, user, ROUTE)?;
            add(dir, user, NO_LIST)?;
            add(dir, user, &format!("{DENY},{DENY_NEW_ENTRIES}"))?;
        }
        // The allow entry also marks the target as routed, so later calls for other targets
        // don't deny it as a sibling.
        let marker = if target.is_dir() { ROUTE } else { FILE_ROUTE };
        if !open || owned(target) {
            add(target, user, marker)?;
        }

        for dir in &chain[start..] {
            let mut deny = Vec::new();
            for entry in
                std::fs::read_dir(dir).with_context(|| format!("list {}", dir.display()))?
            {
                let path = entry?.path();
                if !on_route(&path) && (!open || owned(&path)) && !has_allow(&path, user)? {
                    deny.push(path);
                }
            }
            let deny: Vec<&Path> = deny.iter().map(PathBuf::as_path).collect();
            for chunk in deny.chunks(200) {
                if !chmod(&["-h", "+a", &entry(user, DENY)], chunk)? && open {
                    bail!(
                        "couldn't keep {user} out of the folders next to {} (does this drive support access control lists?)",
                        target.display()
                    );
                }
            }
        }
    }
    Ok(())
}

/// Every entry kind worki adds, for removal. `chmod -a` needs the exact text.
fn all_entry_kinds() -> Vec<String> {
    vec![
        FULL_ACCESS.into(),
        READ_ONLY.into(),
        READ_WRITE_FILE.into(),
        ROUTE.into(),
        NO_LIST.into(),
        FILE_ROUTE.into(),
        DENY.into(),
        format!("{DENY},{DENY_NEW_ENTRIES}"),
        DENY_MODIFY.into(),
    ]
}

/// Removes all of worki's entries for `user` from `path` and, if `recursive`, below it.
pub fn purge(path: &Path, user: &str, recursive: bool) -> Result<()> {
    let flags = if recursive { "-R" } else { "-h" };
    for kind in all_entry_kinds() {
        chmod(&[flags, "-a", &entry(user, &kind)], &[path])?;
    }
    Ok(())
}

/// Follows the traverse-only routes from `dir` and removes every entry for `user` on them, on
/// the entries next to them, and inside granted trees.
pub fn purge_routes(dir: &Path, user: &str) -> Result<()> {
    if !has(dir, user, |_| true)? {
        return Ok(());
    }
    let needle = entry(user, "");
    if let Ok(children) = std::fs::read_dir(dir) {
        for child in children.flatten().map(|e| e.path()) {
            let lines = entries(&child)?;
            let mine: Vec<&String> = lines.iter().filter(|l| l.contains(&needle)).collect();
            if mine.is_empty() {
                continue;
            }
            let routed = mine
                .iter()
                .any(|l| same_rights(l, ROUTE) && !l.contains("add_file"));
            let granted = mine
                .iter()
                .any(|l| same_rights(l, READ_ONLY) || same_rights(l, FULL_ACCESS));
            let real_dir = child.is_dir() && !child.is_symlink();
            if real_dir && routed && !granted {
                purge_routes(&child, user)?;
            } else {
                purge(&child, user, real_dir && granted)?;
            }
        }
    }
    purge(dir, user, false)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn lines(path: &Path) -> Vec<String> {
        entries(path).unwrap()
    }

    #[test]
    fn matches_rights_as_printed_for_directories() {
        let line = "0: user:x allow list,add_file,search,delete,add_subdirectory,delete_child,readattr,writeattr,readextattr,writeextattr,readsecurity,file_inherit,directory_inherit";
        assert!(same_rights(line, FULL_ACCESS));
        assert!(same_rights(line, READ_ONLY));
        assert!(!same_rights("0: user:x allow search", FULL_ACCESS));
        assert!(!same_rights(line, "deny search"));
    }

    #[test]
    fn grants_and_revokes_inheritable_access() {
        let me = crate::current_user();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/f"), "x").unwrap();

        grant_tree(dir.path(), &me, READ_ONLY).unwrap();
        assert!(has_allow(&dir.path().join("sub/f"), &me).unwrap());
        assert!(!has_full(dir.path(), &me).unwrap());

        grant_tree(dir.path(), &me, FULL_ACCESS).unwrap();
        assert!(has_full(dir.path(), &me).unwrap());
        std::fs::write(dir.path().join("sub/new"), "y").unwrap();
        assert!(has_allow(&dir.path().join("sub/new"), &me).unwrap());

        let before = lines(dir.path());
        grant_tree(dir.path(), &me, FULL_ACCESS).unwrap();
        assert_eq!(lines(dir.path()), before, "granting again changes nothing");

        revoke_tree(dir.path(), &me, FULL_ACCESS).unwrap();
        assert!(!has_full(dir.path(), &me).unwrap());
        assert!(!lines(&dir.path().join("sub/new"))
            .iter()
            .any(|l| l.contains("add_subdirectory")));
    }

    /// A repository in folders everyone can enter (a 755 home, `/Users/Shared`): the folders
    /// next to it are still denied.
    #[test]
    fn confines_targets_in_open_folders() {
        let root = tempfile::tempdir().unwrap();
        for dir in [root.path().to_path_buf(), root.path().join("shared")] {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let shared = root.path().join("shared");
        for d in [
            "app/.git",
            "other-repo",
            "worktrees/app/a",
            "worktrees/app/b",
        ] {
            std::fs::create_dir_all(shared.join(d)).unwrap();
        }
        std::fs::write(shared.join("notes.txt"), "secret").unwrap();

        let targets = [shared.join("app/.git"), shared.join("worktrees/app/a")];
        confine_below(&targets, "nobody", root.path()).unwrap();

        let denied = |p: &str| {
            lines(&shared.join(p)).iter().any(|l| {
                l.contains("user:nobody deny")
                    && !l.contains("deny list,add_file,add_subdirectory,delete_child")
                    && !l.contains("only_inherit")
            })
        };
        for p in ["notes.txt", "other-repo", "worktrees/app/b"] {
            assert!(denied(p), "{p} should be denied");
        }
        for p in ["app", "app/.git", "worktrees", "worktrees/app/a"] {
            assert!(!denied(p), "{p} should not be denied");
        }
        purge_routes(root.path(), "nobody").unwrap();
    }

    /// Uses `nobody` as the subject so the test user keeps its own access.
    #[test]
    fn confines_to_targets() {
        let root = tempfile::tempdir().unwrap();
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o750)).unwrap();
        for d in [
            "git/app/.git",
            "git/other",
            "worktrees/app/a",
            "worktrees/app/b",
        ] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::write(home.join(".npmrc"), "token").unwrap();

        let targets = [home.join("git/app/.git"), home.join("worktrees/app/a")];
        confine_below(&targets, "nobody", root.path()).unwrap();

        let denied = |p: &str| {
            lines(&home.join(p)).iter().any(|l| {
                l.contains("user:nobody deny")
                    && !l.contains("deny list,add_file,add_subdirectory,delete_child")
                    && !l.contains("only_inherit")
            })
        };
        let unlistable = |p: &str| {
            lines(&home.join(p))
                .iter()
                .any(|l| l.contains("user:nobody deny list,add_file,add_subdirectory,delete_child"))
        };
        let routed = |p: &str| {
            lines(&home.join(p))
                .iter()
                .any(|l| l.contains("user:nobody allow search"))
        };
        assert!(routed("") && routed("git") && routed("git/app") && routed("worktrees/app"));
        assert!(
            unlistable("git")
                && unlistable("git/app")
                && unlistable("worktrees")
                && unlistable("worktrees/app")
        );
        for p in [
            ".npmrc",
            "git/other",
            "worktrees/app/b",
            "git/app/.git/../../other",
        ] {
            assert!(denied(p), "{p} should be denied");
        }
        for p in ["git", "git/app/.git", "worktrees/app/a"] {
            assert!(!denied(p), "{p} should not be denied");
        }

        std::fs::create_dir(home.join("new-folder")).unwrap();
        assert!(
            lines(&home.join("new-folder"))
                .iter()
                .any(|l| l.contains("user:nobody inherited deny")),
            "new entries inherit the deny"
        );

        confine_below(&[home.join("worktrees/app/b")], "nobody", root.path()).unwrap();
        assert!(!denied("worktrees/app/b"));
        assert!(!denied("worktrees/app/a"), "earlier routes stay open");

        grant_tree(&home.join("git/app/.git"), "nobody", READ_ONLY).unwrap();
        std::fs::write(home.join("git/app/.git/HEAD"), "ref").unwrap();
        purge_routes(&home, "nobody").unwrap();
        for p in [
            "",
            ".npmrc",
            "git",
            "git/other",
            "git/app",
            "git/app/.git",
            "git/app/.git/HEAD",
            "worktrees/app/a",
            "new-folder",
        ] {
            assert!(
                !lines(&home.join(p))
                    .iter()
                    .any(|l| l.contains("user:nobody")),
                "{p} still has entries: {:?}",
                lines(&home.join(p))
            );
        }
    }
}
