use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

use crate::none::shell_args;
use crate::ssh::shell_quote;
use crate::{
    acl, canonical, check_output, command_path, current_user, ensure_relative, ensure_valid_env,
    file_contents, home_path, pack_home_files, run_with_stdin, vault, Backend, Capabilities, Check,
    Fix, Isolation, Launch, LaunchRequest, Workspace, READ_FILE_SCRIPT, WRITE_FILE_SCRIPT,
};

pub const DEFAULT_AGENT_USER: &str = "_worki";
pub const AGENT_GROUP: &str = "_worki_agents";

const AGENT_SHELL: &str = "/bin/zsh";
const SYSTEM_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";
const ADMIN_SCRIPT: &str = include_str!("worki-admin.sh");

/// Sourced from the agent's `~/.zprofile`; rewritten by every `prepare`, so it can evolve.
const AGENT_PROFILE: &str = r#"# Managed by worki; rewritten on setup.
if [ -x /opt/homebrew/bin/brew ]; then eval "$(/opt/homebrew/bin/brew shellenv)"; fi
if [ -x /usr/local/bin/brew ]; then eval "$(/usr/local/bin/brew shellenv)"; fi
export PATH="$HOME/.local/bin:$PATH"
"#;

const SOURCE_PROFILE: &str = r#"[ -f "$HOME/.worki/profile.sh" ] && . "$HOME/.worki/profile.sh""#;

/// Runs agents as a dedicated, hidden macOS user. The human user may switch to it through a
/// sudoers rule installed once by [`MacUser::provision`]; file access comes from ACLs.
pub struct MacUser {
    user: String,
    human: String,
}

impl MacUser {
    pub fn new(user: String) -> Self {
        Self {
            user,
            human: current_user(),
        }
    }

    fn sudo(&self) -> Command {
        let mut cmd = Command::new("/usr/bin/sudo");
        // worki's own working folder is inside the human's home, which the agent can't enter;
        // tools like git refuse to start in a folder they can't read.
        cmd.current_dir("/")
            .env_clear()
            .env("PATH", SYSTEM_PATH)
            .args(["-n", "-u", &self.user, "-H", "--"]);
        cmd
    }

    fn sudo_output(&self, args: &[&str]) -> Option<Output> {
        self.sudo().args(args).stdin(Stdio::null()).output().ok()
    }

    fn sudo_ok(&self, args: &[&str]) -> bool {
        self.sudo_output(args).is_some_and(|o| o.status.success())
    }

    fn can_sudo(&self) -> bool {
        self.sudo_ok(&["/usr/bin/true"])
    }

    fn user_exists(&self) -> bool {
        Command::new("/usr/bin/id")
            .args(["-u", &self.user])
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn agent_git(&self, args: &[&str]) -> Option<String> {
        let mut full = vec!["/usr/bin/git", "config", "--global"];
        full.extend_from_slice(args);
        self.sudo_output(&full)
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    }

    fn is_prepared(&self) -> bool {
        self.agent_git(&["--get-all", "safe.directory"])
            .is_some_and(|v| v.lines().any(|l| l == "*"))
    }

    /// Earlier versions kept a keychain password for the agent user here; removal still clears it.
    fn vault_key(&self) -> String {
        format!("keychain:{}", self.user)
    }

    /// Earlier versions granted the whole repository and worktree folder recursively.
    fn revoke_legacy_grants(&self, ws: &Workspace) -> Result<()> {
        for path in [&ws.repo, &ws.worktree_root] {
            if path.exists() && acl::has_full(path, &self.user)? {
                acl::revoke_tree(path, &self.user, acl::FULL_ACCESS)?;
            }
        }
        Ok(())
    }

    fn can_read(&self, path: &Path) -> bool {
        self.sudo_ok(&["/bin/test", "-r", &path.to_string_lossy()])
    }

    fn can_list(&self, path: &Path) -> bool {
        self.sudo_ok(&["/bin/ls", &path.to_string_lossy()])
    }

    fn can_write(&self, path: &Path) -> bool {
        self.sudo_ok(&["/bin/test", "-w", &path.to_string_lossy()])
    }

    /// Runs a small `sh` script as the agent user in its home, with `args` as `$1…`.
    fn sh(&self, script: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Output> {
        let mut cmd = self.sudo();
        cmd.args(["/bin/sh", "-c", &format!("cd \"$HOME\" && {script}"), "_"])
            .args(args);
        run_with_stdin(&mut cmd, stdin)
    }
}

/// macOS refuses deleting accounts when the request comes from an app without Full Disk
/// Access (dev builds never keep it), even as root, while the same command run from Terminal
/// works. So the deletion is handed to Terminal: a `.command` script runs the admin script with
/// `sudo` (the user types their password there), and worki waits until the account is gone.
fn remove_account_in_terminal(
    user: &str,
    timeout: Duration,
    exists: impl Fn() -> bool,
) -> Result<()> {
    let dir = admin_dir()?;
    let script = write_private(&dir.join("worki-admin.sh"), ADMIN_SCRIPT)?;
    let command_file = write_private(
        &dir.join(format!("remove-{user}.command")),
        &removal_command_script(&script, user),
    )?;

    let opened = Command::new("/usr/bin/open")
        .args(["-a", "Terminal"])
        .arg(&command_file)
        .status()?;
    if !opened.success() {
        bail!("could not open Terminal; run `sudo sysadminctl -deleteUser {user}` there yourself");
    }
    if !wait_while(exists, timeout, Duration::from_secs(1)) {
        bail!("the account {user} still exists; finish the removal in the Terminal window worki opened");
    }
    Ok(())
}

/// Polls until `condition` is false; returns whether that happened before `timeout`.
fn wait_while(condition: impl Fn() -> bool, timeout: Duration, every: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while condition() {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(every);
    }
    true
}

fn removal_command_script(admin_script: &Path, user: &str) -> String {
    let admin = shell_quote(&admin_script.to_string_lossy());
    let quoted_user = shell_quote(user);
    format!(
        r#"#!/bin/sh
clear
echo "worki: removing the agent user {user}."
echo "macOS only allows deleting accounts from Terminal, so enter your password here."
echo
sudo /bin/sh {admin} remove {quoted_user}
status=$?
rm -f {admin} "$0"
echo
if [ "$status" -eq 0 ]; then echo "Done. You can close this window."; else echo "Removal failed (exit $status)."; fi
"#
    )
}

fn admin_dir() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("worki-admin-{}", current_user()));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir)
}

/// Writes an executable only the owner can read, write or run.
fn write_private(path: &Path, contents: &str) -> Result<PathBuf> {
    std::fs::write(path, contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(path.to_path_buf())
}

/// Runs the bundled admin script as root through the system authentication dialog.
fn run_admin(args: &[&str], prompt: &str) -> Result<()> {
    let script = write_private(&admin_dir()?.join("worki-admin.sh"), ADMIN_SCRIPT)?;
    let mut command = format!(
        "\"/bin/sh \" & quoted form of {}",
        applescript_string(&script.to_string_lossy())
    );
    for arg in args {
        command.push_str(&format!(
            " & \" \" & quoted form of {}",
            applescript_string(arg)
        ));
    }
    let apple = format!(
        "do shell script {command} with administrator privileges with prompt {}",
        applescript_string(prompt)
    );
    let out = Command::new("/usr/bin/osascript")
        .args(["-e", &apple])
        .output()?;
    std::fs::remove_file(&script).ok();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("-128") {
            bail!("Cancelled.");
        }
        bail!("{}", err.trim());
    }
    Ok(())
}

/// The `.git/worktrees/<name>` folder a linked worktree's `.git` file points to.
fn worktree_admin_dir(worktree: &Path) -> Result<PathBuf> {
    let gitfile = std::fs::read_to_string(worktree.join(".git"))
        .with_context(|| format!("{} is not a linked worktree", worktree.display()))?;
    let dir = gitfile
        .strip_prefix("gitdir:")
        .map(str::trim)
        .with_context(|| format!("unexpected .git file in {}", worktree.display()))?;
    canonical(&worktree.join(dir))
}

fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Backend for MacUser {
    fn isolation(&self) -> Isolation {
        Isolation::MacUser {
            user: self.user.clone(),
        }
    }

    /// `sudo` resets the environment, so variables are handed over by name through
    /// `--preserve-env` from sudo's own (otherwise empty) environment. Values never appear in
    /// argv, where other users could read them with `ps`.
    fn launch(&self, req: &LaunchRequest) -> Result<Launch> {
        ensure_valid_env(&req.env)?;
        let mut args: Vec<String> = vec!["-n".into(), "-u".into(), self.user.clone(), "-H".into()];
        if !req.env.is_empty() {
            let names: Vec<&str> = req.env.iter().map(|(k, _)| k.as_str()).collect();
            args.push(format!("--preserve-env={}", names.join(",")));
        }
        args.push("--".into());
        args.push(AGENT_SHELL.into());
        args.extend(shell_args(req));

        let mut env = req.env.clone();
        env.push(("PATH".into(), SYSTEM_PATH.into()));
        Ok(Launch {
            program: "/usr/bin/sudo".into(),
            args,
            env,
            clear_env: true,
            cwd: req.cwd.clone(),
        })
    }

    /// Confines the agent to the assigned worktree plus what git needs from the repository:
    /// read access to `.git`, write access only where commits land and to the worktree's own
    /// admin folder. `config` and `hooks` stay read-only so nothing the agent writes runs as you.
    fn grant_access(&self, ws: &Workspace) -> Result<()> {
        let user = &self.user;
        let git_dir = canonical(&ws.repo.join(".git"))?;
        if !git_dir.is_dir() {
            bail!("{} has no .git folder", ws.repo.display());
        }
        self.revoke_legacy_grants(ws)?;

        let linked = match &ws.worktree {
            Some(worktree) => {
                let worktree = crate::linked_worktree(&ws.repo, worktree)?;
                let admin = worktree_admin_dir(&worktree)?;
                Some((worktree, admin))
            }
            None => None,
        };
        let mut targets = vec![git_dir.clone()];
        targets.extend(linked.as_ref().map(|(worktree, _)| worktree.clone()));
        acl::confine(&targets, user)?;

        acl::grant_tree(&git_dir, user, acl::READ_ONLY)?;
        for sub in ["objects", "refs", "logs"] {
            let dir = git_dir.join(sub);
            std::fs::create_dir_all(&dir)?;
            acl::grant_tree(&dir, user, acl::FULL_ACCESS)?;
            acl::grant_tree(&dir, &self.human, acl::FULL_ACCESS)?;
        }
        let fetch_head = git_dir.join("FETCH_HEAD");
        if !fetch_head.exists() {
            std::fs::write(&fetch_head, "")?;
        }
        acl::grant_file(&fetch_head, user, acl::READ_WRITE_FILE)?;

        if let Some((worktree, admin)) = linked {
            for dir in [&worktree, &admin] {
                acl::grant_tree(dir, user, acl::FULL_ACCESS)?;
                acl::grant_tree(dir, &self.human, acl::FULL_ACCESS)?;
            }
            // These files tell git where the repository lives; rewriting them could point your
            // own git at config the agent controls.
            for pointer in [
                worktree.join(".git"),
                admin.join("gitdir"),
                admin.join("commondir"),
            ] {
                if pointer.exists() {
                    acl::deny_modify(&pointer, user)?;
                }
            }
        }
        Ok(())
    }

    fn prepare(&self) -> Result<()> {
        if !self.is_prepared() {
            self.agent_git(&["--add", "safe.directory", "*"])
                .context("configure git safe.directory for the agent user")?;
        }
        for key in ["user.name", "user.email"] {
            let mine = Command::new("/usr/bin/git")
                .args(["config", "--global", "--get", key])
                .output()?;
            let value = String::from_utf8_lossy(&mine.stdout).trim().to_owned();
            if !value.is_empty()
                && self
                    .agent_git(&["--get", key])
                    .unwrap_or_default()
                    .is_empty()
            {
                self.agent_git(&[key, &value]);
            }
        }
        self.write_home_file(Path::new(".worki/profile.sh"), AGENT_PROFILE.as_bytes())?;
        let out = self.sh(
            r#"touch .zprofile && { grep -qF "$1" .zprofile || printf '\n%s\n' "$1" >> .zprofile; }"#,
            &[SOURCE_PROFILE],
            None,
        )?;
        check_output(out, "updating .zprofile").map(drop)
    }

    fn provision(&self) -> Result<()> {
        run_admin(
            &["setup", &self.human, &self.user],
            &format!("worki wants to create the agent user “{}”.", self.user),
        )?;
        self.prepare()
    }

    /// Permissions go first: once the account is deleted, its entries can no longer be named.
    fn remove(&self, workspaces: &[Workspace]) -> Result<()> {
        if !self.user_exists() {
            return vault::delete(&self.vault_key());
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            acl::purge_routes(&home, &self.user)?;
        }
        for ws in workspaces {
            for path in [ws.repo.join(".git"), ws.worktree_root.clone()] {
                if path.exists() {
                    acl::purge(&path, &self.user, true)?;
                }
            }
            for dir in ws.repo.ancestors().chain(ws.worktree_root.ancestors()) {
                if dir.exists() {
                    acl::purge_routes(dir, &self.user)?;
                }
            }
        }
        vault::delete(&self.vault_key())?;
        remove_account_in_terminal(&self.user, Duration::from_secs(600), || self.user_exists())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            share_host_install: true,
            install: true,
        }
    }

    /// Removes the agent user's entries on the project's repository and worktree folder and
    /// everything below. Shared parent folders keep theirs: other projects still route through.
    fn forget_project(&self, ws: &Workspace) -> Result<()> {
        if !self.user_exists() {
            return Ok(());
        }
        for dir in [&ws.repo, &ws.worktree_root] {
            if dir.exists() {
                acl::purge_routes(dir, &self.user)?;
                acl::purge(dir, &self.user, false)?;
            }
        }
        Ok(())
    }

    fn ready(&self) -> Result<()> {
        if !self.user_exists() {
            bail!("User Isolation isn't set up: the user {} doesn't exist. Open Isolation to set it up or choose None.", self.user);
        }
        if !self.can_sudo() {
            bail!("User Isolation isn't set up: you can't run commands as {} (the sudoers rule is missing). Open Isolation to fix it.", self.user);
        }
        Ok(())
    }

    fn exists(&self) -> bool {
        self.user_exists()
    }

    fn which(&self, command: &str) -> Option<String> {
        self.sudo_output(&[
            AGENT_SHELL,
            "-l",
            "-i",
            "-c",
            "command -v \"$1\"",
            "_",
            command,
        ])
        .and_then(|o| command_path(&o))
    }

    fn home(&self) -> Option<PathBuf> {
        home_path(&self.sudo_output(&["/bin/sh", "-c", "echo \"$HOME\""])?)
    }

    /// Grants read-only access to the install (the directory holding the resolved binary, so
    /// self-updates that add versions next to it stay readable) and links it into
    /// `~/.local/bin`, which the agent's `.zprofile` puts on PATH.
    fn share_host_executable(&self, name: &str, host_path: &Path) -> Result<()> {
        ensure_relative(Path::new(name))?;
        let resolved = canonical(host_path)?;
        let install_dir = resolved
            .parent()
            .context("executable has no parent folder")?
            .to_path_buf();
        acl::confine(&[host_path.to_path_buf(), install_dir.clone()], &self.user)?;
        acl::grant_tree(&install_dir, &self.user, acl::READ_ONLY)?;
        let out = self.sh(
            r#"mkdir -p .local/bin && ln -sfn "$1" ".local/bin/$2""#,
            &[&host_path.to_string_lossy(), name],
            None,
        )?;
        check_output(out, "linking the executable").map(drop)
    }

    /// Streams a tar of the files from the human side into a tar extracting as the agent user,
    /// so neither side needs access to the other's home.
    fn copy_home_files(&self, from_home: &Path, paths: &[PathBuf]) -> Result<()> {
        for p in paths {
            ensure_relative(p)?;
        }
        if paths.is_empty() {
            return Ok(());
        }
        let mut pack = pack_home_files(from_home, paths)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("tar")?;
        let tar_out = pack.stdout.take().context("tar stdout")?;
        let unpack = self
            .sudo()
            .args([
                "/bin/sh",
                "-c",
                r#"cd "$HOME" && /usr/bin/tar --no-acls --no-xattrs --no-same-owner -xf -"#,
            ])
            .stdin(tar_out)
            .stderr(Stdio::piped())
            .output()?;
        // Unreadable or dangling entries make the packing tar exit non-zero but still archive the
        // rest; a partial copy is more useful than none, so only the unpacking side is fatal.
        pack.wait()?;
        check_output(unpack, "writing the agent user's files").map(drop)
    }

    fn read_home_file(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        ensure_relative(path)?;
        let out = self.sh(READ_FILE_SCRIPT, &[&path.to_string_lossy()], None)?;
        file_contents(out, &format!("reading {}", path.display()))
    }

    fn write_home_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        ensure_relative(path)?;
        let out = self.sh(WRITE_FILE_SCRIPT, &[&path.to_string_lossy()], Some(data))?;
        check_output(out, &format!("writing {}", path.display())).map(drop)
    }

    fn doctor(&self, ws: Option<&Workspace>) -> Vec<Check> {
        let mut checks = Vec::new();
        let exists = self.user_exists();
        checks.push(
            Check::new(
                "user",
                exists,
                format!("Agent user {} exists", self.user),
                format!("Agent user {} does not exist", self.user),
            )
            .fix_if_failed(Fix::Provision),
        );
        if !exists {
            return checks;
        }

        let sudo = self.can_sudo();
        checks.push(
            Check::new(
                "sudo",
                sudo,
                format!("You can run commands as {}", self.user),
                format!("You can't run commands as {}", self.user),
            )
            .detail(if sudo {
                "Passwordless via /etc/sudoers.d/worki"
            } else {
                "The sudoers rule is missing"
            })
            .fix_if_failed(Fix::Provision),
        );
        if !sudo {
            return checks;
        }

        let group = Command::new("/usr/bin/id")
            .args(["-gn", &self.user])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
            .unwrap_or_default();
        checks.push(
            Check::new(
                "group",
                group == AGENT_GROUP,
                format!("{} has its own group", self.user),
                format!("{} shares your group ({group})", self.user),
            )
            .detail(if group == AGENT_GROUP {
                format!("Primary group {AGENT_GROUP}.")
            } else {
                format!("Members of {group} can read your home folder. Set up moves {} to {AGENT_GROUP}.", self.user)
            })
            .fix_if_failed(Fix::Provision),
        );
        checks.push(
            Check::new(
                "prepared",
                self.is_prepared(),
                "Agent git config is set up",
                "Agent git config is missing",
            )
            .fix_if_failed(Fix::Prepare),
        );

        if let Some(ws) = ws {
            let objects = ws.repo.join(".git/objects");
            let ok = self.can_write(&objects) && !self.can_write(&ws.repo.join(".git/config"));
            checks.push(
                Check::new(
                    "access",
                    ok,
                    format!("{} can commit to this repository", self.user),
                    format!("{} can't commit to this repository", self.user),
                )
                .detail(
                    "Write access to .git objects and refs only; config and hooks stay read-only.",
                )
                .fix_if_failed(Fix::GrantAccess),
            );
            let leaks: Vec<String> = [
                ws.repo.clone(),
                ws.repo.parent().map(Path::to_path_buf).unwrap_or_default(),
            ]
            .iter()
            .filter(|p| !p.as_os_str().is_empty() && self.can_list(p))
            .map(|p| p.display().to_string())
            .collect();
            checks.push(
                Check::new(
                    "confined",
                    leaks.is_empty(),
                    format!("Your other files are hidden from {}", self.user),
                    format!("Your other files are not hidden from {}", self.user),
                )
                .detail(if leaks.is_empty() {
                    "Only assigned worktrees and the repository's .git are reachable.".to_string()
                } else {
                    format!("{} can list: {}", self.user, leaks.join(", "))
                })
                .fix_if_failed(Fix::GrantAccess),
            );
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let exposed: Vec<&str> = [".gitconfig", ".zshrc", ".bashrc", ".npmrc", ".config"]
                .into_iter()
                .filter(|f| home.join(f).exists() && self.can_read(&home.join(f)))
                .collect();
            checks.push(
                Check::new(
                    "home",
                    exposed.is_empty(),
                    format!("Your home folder is protected from {}", self.user),
                    format!("Your home folder is not protected from {}", self.user),
                )
                .detail(if exposed.is_empty() {
                    "Checked common dotfiles.".to_string()
                } else {
                    format!("{} can read: {}", self.user, exposed.join(", "))
                })
                .fix_if_failed(Fix::GrantAccess),
            );
        }
        checks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backend() -> MacUser {
        MacUser {
            user: "_worki_test_no_such_user".into(),
            human: "me".into(),
        }
    }

    #[test]
    fn launches_through_sudo_without_values_in_argv() {
        let req = LaunchRequest {
            cwd: "/w".into(),
            command: Some("exec claude".into()),
            interactive: true,
            stdin: false,
            env: vec![
                ("TERM".into(), "xterm-256color".into()),
                ("SECRET_TOKEN".into(), "s3cr3t".into()),
            ],
            session: None,
        };
        let l = backend().launch(&req).unwrap();
        assert_eq!(l.program, "/usr/bin/sudo");
        assert_eq!(
            l.args,
            [
                "-n",
                "-u",
                "_worki_test_no_such_user",
                "-H",
                "--preserve-env=TERM,SECRET_TOKEN",
                "--",
                "/bin/zsh",
                "-l",
                "-i",
                "-c",
                "exec claude"
            ]
        );
        assert!(l.clear_env);
        assert!(!l.args.iter().any(|a| a.contains("s3cr3t")));
        assert!(l.env.contains(&("SECRET_TOKEN".into(), "s3cr3t".into())));
    }

    #[test]
    fn rejects_env_names_that_could_smuggle_options() {
        let req = LaunchRequest {
            cwd: "/w".into(),
            command: None,
            interactive: true,
            stdin: false,
            env: vec![("A,LD_PRELOAD".into(), "x".into())],
            session: None,
        };
        assert!(backend().launch(&req).is_err());
    }

    #[test]
    fn removal_script_quotes_paths_and_cleans_up() {
        let script = removal_command_script(Path::new("/tmp/it's here/worki-admin.sh"), "_worki");
        assert!(script.contains(r"sudo /bin/sh '/tmp/it'\''s here/worki-admin.sh' remove '_worki'"));
        assert!(script.contains(r#"rm -f '/tmp/it'\''s here/worki-admin.sh' "$0""#));
        let path = std::env::temp_dir().join("worki-removal-syntax.sh");
        std::fs::write(&path, &script).unwrap();
        assert!(Command::new("/bin/sh")
            .arg("-n")
            .arg(&path)
            .status()
            .unwrap()
            .success());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn waits_until_account_is_gone_or_times_out() {
        let calls = std::cell::Cell::new(0);
        let exists_twice = || {
            calls.set(calls.get() + 1);
            calls.get() < 3
        };
        assert!(wait_while(
            exists_twice,
            Duration::from_secs(5),
            Duration::from_millis(1)
        ));
        assert_eq!(calls.get(), 3);
        assert!(!wait_while(
            || true,
            Duration::from_millis(20),
            Duration::from_millis(5)
        ));
    }

    #[test]
    fn quotes_applescript_strings() {
        assert_eq!(applescript_string(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
