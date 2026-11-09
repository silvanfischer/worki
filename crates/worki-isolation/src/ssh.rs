//! Agents on another machine, reached over SSH. The project's repository and worktrees live on
//! that host; worki only runs commands there. With `keep_alive`, sessions run inside a tmux
//! server on the host (its own socket, so the user's tmux is untouched) and survive worki
//! quitting or the connection dropping.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Mutex;

use anyhow::{bail, Result};

use crate::{
    check_output, ensure_relative, file_contents, home_path, run_with_stdin, valid_env_name,
    Backend, Check, Fix, Isolation, Launch, LaunchRequest, Workspace,
};

/// tmux socket name (`tmux -L`) on the remote host.
pub const TMUX_SOCKET: &str = "worki";
pub const TMUX_PREFIX: &str = "worki-";

/// Extra PATH entries for non-login commands: where package managers put git and tmux.
const PATH_PRELUDE: &str =
    "PATH=\"$PATH:/usr/local/bin:/opt/homebrew/bin:/home/linuxbrew/.linuxbrew/bin:$HOME/.local/bin\"; export PATH; ";

const TMUX_CONF: &str = "\
set -gq status off
set -gq mouse on
set -gq history-limit 50000
set -sgq escape-time 0
set -gq default-terminal screen-256color
set -gaq terminal-overrides ,xterm-256color:RGB
set -gq allow-passthrough on
set -gq set-clipboard on
set -gq focus-events on
set -gq aggressive-resize on
set -gq exit-empty on
";

/// Installs tmux with whatever package manager the host has; without root it needs
/// passwordless sudo, otherwise it fails with the command to run by hand.
const INSTALL_TMUX: &str = r#"
command -v tmux >/dev/null 2>&1 && exit 0
if [ "$(id -u)" = 0 ]; then SUDO=; elif sudo -n true 2>/dev/null; then SUDO="sudo -n"; else SUDO=none; fi
if command -v brew >/dev/null 2>&1; then brew install tmux; exit $?; fi
for pm in apt-get dnf yum apk pacman zypper; do
  command -v $pm >/dev/null 2>&1 || continue
  case $pm in
    apt-get) cmd="apt-get update -qq && apt-get install -y -qq tmux" ;;
    dnf|yum) cmd="$pm install -y tmux" ;;
    apk) cmd="apk add --no-cache tmux" ;;
    pacman) cmd="pacman -S --noconfirm tmux" ;;
    zypper) cmd="zypper --non-interactive install tmux" ;;
  esac
  if [ "$SUDO" = none ]; then
    echo "Installing tmux needs root. On the host, run: sudo sh -c '$cmd'" >&2
    exit 1
  fi
  $SUDO sh -c "$cmd"
  exit $?
done
echo "No supported package manager found; install tmux on the host by hand." >&2
exit 1
"#;

/// Where to connect: host (or `~/.ssh/config` alias), optional user and port. Unset values
/// come from the user's SSH config.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshTarget {
    pub host: String,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
}

impl SshTarget {
    pub fn new(host: impl Into<String>, user: Option<String>, port: Option<u16>) -> Self {
        Self {
            host: host.into(),
            user: user.filter(|u| !u.trim().is_empty()),
            port,
        }
    }

    /// `user@host:port`, leaving out what isn't set.
    pub fn display(&self) -> String {
        let mut s = String::new();
        if let Some(user) = &self.user {
            s.push_str(user);
            s.push('@');
        }
        s.push_str(&self.host);
        if let Some(port) = self.port {
            s.push_str(&format!(":{port}"));
        }
        s
    }

    pub fn validate(&self) -> Result<()> {
        if !valid_host(&self.host) {
            bail!("invalid SSH host: {}", self.host);
        }
        if let Some(user) = &self.user {
            let ok = !user.starts_with('-')
                && user.len() <= 64
                && user
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
            if !ok {
                bail!("invalid SSH user name: {user}");
            }
        }
        if self.port == Some(0) {
            bail!("invalid SSH port: 0");
        }
        Ok(())
    }

    fn vault_key(&self) -> String {
        format!("ssh-password:{}", self.display())
    }
}

/// Passwords by vault key, so the Keychain is read at most once per target and run.
static PASSWORDS: Mutex<BTreeMap<String, Option<String>>> = Mutex::new(BTreeMap::new());

fn passwords() -> std::sync::MutexGuard<'static, BTreeMap<String, Option<String>>> {
    PASSWORDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The stored login password for `target`.
pub fn password(target: &SshTarget) -> Option<String> {
    let key = target.vault_key();
    passwords()
        .entry(key.clone())
        .or_insert_with(|| {
            crate::vault::has(&key)
                .then(|| crate::vault::get(&key))
                .flatten()
        })
        .clone()
}

pub fn has_password(target: &SshTarget) -> bool {
    crate::vault::has(&target.vault_key())
}

/// Stores (or with `None` removes) the login password for `target`.
pub fn set_password(target: &SshTarget, password: Option<&str>) -> Result<()> {
    let key = target.vault_key();
    let password = password.filter(|p| !p.is_empty());
    match password {
        Some(p) => crate::vault::set(&key, p)?,
        None => crate::vault::delete(&key)?,
    }
    passwords().insert(key, password.map(str::to_owned));
    Ok(())
}

/// A tiny `SSH_ASKPASS` helper that answers the password prompt from worki's environment.
fn askpass_program() -> Result<PathBuf> {
    let path = home_dir().join(".ssh/worki-askpass");
    let body = "#!/bin/sh\nprintf '%s\\n' \"$WORKI_SSH_PASSWORD\"\n";
    if std::fs::read_to_string(&path).ok().as_deref() != Some(body) {
        ssh_dir();
        std::fs::write(&path, body)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

/// How to run `ssh` for a target: arguments up to and including the destination, and the
/// environment the `ssh` process needs (password answering).
pub struct SshCommand {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl SshCommand {
    pub fn std(&self) -> Command {
        let mut cmd = Command::new("/usr/bin/ssh");
        cmd.args(&self.args)
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

pub struct Ssh {
    target: SshTarget,
    keep_alive: bool,
}

/// Quotes `s` as one word for POSIX shells (and fish, which reads `'\''` the same way).
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Valid `ssh` destinations: a `~/.ssh/config` alias or `[user@]host[:port]`-free hostname.
pub fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 255
        && !host.starts_with('-')
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-@".contains(&b))
}

/// `~/.ssh`, created private if missing.
fn ssh_dir() -> PathBuf {
    let dir = home_dir().join(".ssh");
    if !dir.exists() {
        std::fs::create_dir_all(&dir).ok();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    }
    dir
}

fn control_path() -> String {
    ssh_dir().join("worki-%C").to_string_lossy().into_owned()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

/// `ssh` for `target`. Connections are shared (one login per host for all commands and
/// sessions). `interactive` allocates a terminal. A stored password is answered through
/// `SSH_ASKPASS`; without one, logins must not prompt (keys or ssh-agent).
pub fn ssh_command(target: &SshTarget, interactive: bool) -> SshCommand {
    let mut args: Vec<String> = [
        "-o",
        "ControlMaster=auto",
        "-o",
        &format!("ControlPath={}", control_path()),
        "-o",
        "ControlPersist=600",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut env = Vec::new();
    let password = password(target);
    match (&password, askpass_program()) {
        (Some(pw), Ok(askpass)) => {
            env.push((
                "SSH_ASKPASS".to_owned(),
                askpass.to_string_lossy().into_owned(),
            ));
            env.push(("SSH_ASKPASS_REQUIRE".to_owned(), "force".to_owned()));
            env.push(("WORKI_SSH_PASSWORD".to_owned(), pw.clone()));
            args.extend(["-o", "NumberOfPasswordPrompts=1"].map(String::from));
        }
        _ if !interactive => args.extend(["-o", "BatchMode=yes"].map(String::from)),
        _ => {}
    }
    if interactive {
        args.push("-t".into());
    } else {
        args.extend(["-T", "-o", "ConnectTimeout=10"].map(String::from));
    }
    if let Some(port) = target.port {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    if let Some(user) = &target.user {
        args.extend(["-l".to_owned(), user.clone()]);
    }
    args.push("--".into());
    args.push(target.host.clone());
    SshCommand { args, env }
}

/// The single command string sent to the remote login shell: `sh -c script _ args…`, so the
/// script runs under `sh` whatever the user's shell is.
pub fn remote_sh(script: &str, args: &[&str]) -> String {
    let mut cmd = format!(
        "sh -c {} _",
        shell_quote(&format!("{PATH_PRELUDE}{script}"))
    );
    for a in args {
        cmd.push(' ');
        cmd.push_str(&shell_quote(a));
    }
    cmd
}

/// Shell code exporting `env` (for `. file`).
fn env_file(env: &[(String, String)]) -> String {
    env.iter()
        .filter(|(k, _)| valid_env_name(k))
        .map(|(k, v)| format!("export {k}={}\n", shell_quote(v)))
        .collect()
}

pub fn tmux_name(session_id: &str) -> String {
    format!("{TMUX_PREFIX}{session_id}")
}

/// A worki tmux session found on a host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteSession {
    pub id: String,
    pub worktree: PathBuf,
    pub title: String,
    pub profile_id: Option<String>,
}

pub fn parse_tmux_sessions(text: &str) -> Vec<RemoteSession> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let id = f.next()?.strip_prefix(TMUX_PREFIX)?.to_owned();
            let worktree = PathBuf::from(f.next().filter(|s| !s.is_empty())?);
            let title = f
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Session")
                .to_owned();
            let profile_id = f.next().filter(|s| !s.is_empty()).map(str::to_owned);
            Some(RemoteSession {
                id,
                worktree,
                title,
                profile_id,
            })
        })
        .collect()
}

impl Ssh {
    pub fn new(target: SshTarget, keep_alive: bool) -> Self {
        Self { target, keep_alive }
    }

    fn name(&self) -> String {
        self.target.display()
    }

    /// Runs `sh -c script` on the host with `args` as `$1…`, optionally feeding stdin.
    pub fn sh(&self, script: &str, args: &[&str], stdin: Option<&[u8]>) -> Result<Output> {
        let mut cmd = ssh_command(&self.target, false).std();
        cmd.arg(remote_sh(script, args));
        run_with_stdin(&mut cmd, stdin)
    }

    fn checked(
        &self,
        script: &str,
        args: &[&str],
        stdin: Option<&[u8]>,
        what: &str,
    ) -> Result<String> {
        let out = check_output(
            self.sh(script, args, stdin)?,
            &format!("{what} on {}", self.name()),
        )?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn connect_error(&self) -> Option<String> {
        match self.sh("true", &[], None) {
            Ok(out) if out.status.success() => None,
            Ok(out) => Some(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
            Err(e) => Some(e.to_string()),
        }
    }

    fn has(&self, command: &str) -> bool {
        self.sh("command -v \"$1\" >/dev/null 2>&1", &[command], None)
            .is_ok_and(|o| o.status.success())
    }

    /// Writes `env` to a private file on the host and returns its path; the launched script
    /// sources and deletes it, so secrets never appear in a command line.
    fn stage_env(&self, env: &[(String, String)]) -> Result<String> {
        let name = format!("{:016x}", rand_u64());
        self.checked(
            "umask 077 && mkdir -p \"$HOME/.worki/env\" && cat > \"$HOME/.worki/env/$1\" && printf %s \"$HOME/.worki/env/$1\"",
            &[&name],
            Some(env_file(env).as_bytes()),
            "passing the environment",
        )
    }

    /// Program and args that attach to the kept-alive session `id` on the host.
    pub fn attach_launch(&self, session_id: &str) -> Launch {
        let script = format!(
            "exec tmux -L {TMUX_SOCKET} attach-session -t {}",
            shell_quote(&tmux_name(session_id))
        );
        self.interactive_launch(&script, home_dir())
    }

    fn interactive_launch(&self, script: &str, cwd: PathBuf) -> Launch {
        let SshCommand { mut args, mut env } = ssh_command(&self.target, true);
        args.push(remote_sh(script, &[]));
        env.extend([
            ("TERM".to_owned(), "xterm-256color".to_owned()),
            ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ]);
        Launch {
            program: "/usr/bin/ssh".into(),
            args,
            env,
            clear_env: false,
            cwd,
        }
    }

    /// worki tmux sessions on the host. Empty when tmux or its server isn't there.
    pub fn sessions(&self) -> Result<Vec<RemoteSession>> {
        let listing = self.checked(
            &format!(
                "command -v tmux >/dev/null 2>&1 || exit 0; tmux -L {TMUX_SOCKET} list-sessions -F \
                 '#{{session_name}}\t#{{@worki_worktree}}\t#{{@worki_title}}\t#{{@worki_profile}}' 2>/dev/null; exit 0"
            ),
            &[],
            None,
            "listing sessions",
        )?;
        Ok(parse_tmux_sessions(&listing))
    }

    pub fn session_exists(&self, session_id: &str) -> bool {
        self.sh(
            &format!("tmux -L {TMUX_SOCKET} has-session -t \"$1\" 2>/dev/null"),
            &[&tmux_name(session_id)],
            None,
        )
        .is_ok_and(|o| o.status.success())
    }

    pub fn kill_session(&self, session_id: &str) -> Result<()> {
        self.sh(
            &format!("tmux -L {TMUX_SOCKET} kill-session -t \"$1\" 2>/dev/null; exit 0"),
            &[&tmux_name(session_id)],
            None,
        )?;
        Ok(())
    }

    fn install_tmux(&self) -> Result<()> {
        self.checked(INSTALL_TMUX, &[], None, "installing tmux")
            .map(drop)
    }
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    h.finish()
}

impl Backend for Ssh {
    fn isolation(&self) -> Isolation {
        Isolation::Ssh {
            host: self.target.host.clone(),
            user: self.target.user.clone(),
            port: self.target.port,
            keep_alive: self.keep_alive,
        }
    }

    fn launch(&self, req: &LaunchRequest) -> Result<Launch> {
        let env_path = self.stage_env(&req.env)?;
        let shell_flags = if req.interactive { "-l -i" } else { "-l" };
        let run = match &req.command {
            Some(command) => format!(
                "exec \"${{SHELL:-/bin/sh}}\" {shell_flags} -c {}",
                shell_quote(command)
            ),
            None => "exec \"${SHELL:-/bin/sh}\" -l".to_owned(),
        };
        let cwd = req.cwd.to_string_lossy();
        let inner = format!(
            "cd {} || exit 1; set -a; . {env}; set +a; rm -f {env}; {run}",
            shell_quote(&cwd),
            env = shell_quote(&env_path),
        );

        let Some(session) = req
            .session
            .as_ref()
            .filter(|_| self.keep_alive && req.interactive)
        else {
            if req.interactive {
                return Ok(self.interactive_launch(&inner, home_dir()));
            }
            let SshCommand { mut args, env } = ssh_command(&self.target, false);
            args.push(remote_sh(&inner, &[]));
            return Ok(Launch {
                program: "/usr/bin/ssh".into(),
                args,
                env,
                clear_env: false,
                cwd: home_dir(),
            });
        };

        if !self.has("tmux") {
            self.install_tmux()?;
        }
        let name = shell_quote(&tmux_name(&session.id));
        let conf = shell_quote(TMUX_CONF);
        let script = format!(
            "mkdir -p \"$HOME/.worki\" && printf %s {conf} > \"$HOME/.worki/tmux.conf\" && \
             tmux -L {TMUX_SOCKET} -f \"$HOME/.worki/tmux.conf\" new-session -d -s {name} -c {cwd_q} {inner_q} && \
             tmux -L {TMUX_SOCKET} set-option -t {name} @worki_worktree {cwd_q} >/dev/null && \
             tmux -L {TMUX_SOCKET} set-option -t {name} @worki_title {title} >/dev/null && \
             tmux -L {TMUX_SOCKET} set-option -t {name} @worki_profile {profile} >/dev/null; \
             exec tmux -L {TMUX_SOCKET} attach-session -t {name}",
            cwd_q = shell_quote(&cwd),
            inner_q = shell_quote(&format!("sh -c {}", shell_quote(&format!("{PATH_PRELUDE}{inner}")))),
            title = shell_quote(&session.title),
            profile = shell_quote(session.profile_id.as_deref().unwrap_or("")),
        );
        Ok(self.interactive_launch(&script, home_dir()))
    }

    fn grant_access(&self, _ws: &Workspace) -> Result<()> {
        Ok(())
    }

    fn ready(&self) -> Result<()> {
        match self.connect_error() {
            None => Ok(()),
            Some(e) => bail!("can't connect to {}: {e}", self.name()),
        }
    }

    fn provision(&self) -> Result<()> {
        self.ready()?;
        if self.keep_alive {
            self.install_tmux()?;
        }
        Ok(())
    }

    fn doctor(&self, ws: Option<&Workspace>) -> Vec<Check> {
        let mut checks = Vec::new();
        let error = self.connect_error();
        checks.push(
            Check::new(
                "connect",
                error.is_none(),
                format!("worki can connect to {}", self.name()),
                format!("worki can't connect to {}", self.name()),
            )
            .detail(match &error {
                None => "Uses your ~/.ssh/config and keys; the connection is shared by all sessions.".to_owned(),
                Some(e) if has_password(&self.target) => format!("{e}\nCheck the user name, port and password in the project settings."),
                Some(e) => format!(
                    "{e}\nworki can't answer prompts here: use a key (ssh-agent or a key without passphrase) or store a password in the project settings."
                ),
            }),
        );
        if error.is_some() {
            return checks;
        }
        let git = self.has("git");
        checks.push(Check::new(
            "git",
            git,
            format!("git is installed on {}", self.name()),
            format!("git is not installed on {}", self.name()),
        ));
        if self.keep_alive {
            let tmux = self.has("tmux");
            checks.push(
                Check::new(
                    "tmux",
                    tmux,
                    format!("tmux is installed on {}", self.name()),
                    format!("tmux is not installed on {}", self.name()),
                )
                .detail(if tmux {
                    "Sessions keep running on the host when worki quits or the connection drops."
                } else {
                    "Needed to keep sessions alive. worki installs it with the host's package manager (root or passwordless sudo)."
                })
                .fix_if_failed(Fix::Provision),
            );
        }
        if let Some(ws) = ws {
            let repo = ws.repo.to_string_lossy();
            let found = git
                && self
                    .sh(
                        "git -C \"$1\" rev-parse --git-dir >/dev/null 2>&1",
                        &[&repo],
                        None,
                    )
                    .is_ok_and(|o| o.status.success());
            checks.push(Check::new(
                "repo",
                found,
                format!("The repository exists at {repo}"),
                format!("There is no repository at {repo}"),
            ));
        }
        checks
    }

    fn which(&self, command: &str) -> Option<String> {
        let out = self
            .sh(
                "exec \"${SHELL:-/bin/sh}\" -l -c 'command -v \"$0\"' \"$1\" 2>/dev/null",
                &[command],
                None,
            )
            .ok()?;
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .rev()
            .map(str::trim)
            .find(|l| l.starts_with('/'))
            .map(str::to_owned)
    }

    fn home(&self) -> Option<PathBuf> {
        home_path(&self.sh("printf %s \"$HOME\"", &[], None).ok()?)
    }

    fn copy_home_files(&self, from_home: &Path, paths: &[PathBuf]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        for p in paths {
            ensure_relative(p)?;
        }
        let tar = Command::new("/usr/bin/tar")
            .arg("-C")
            .arg(from_home)
            .arg("-cf")
            .arg("-")
            .args(paths)
            .env("COPYFILE_DISABLE", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;
        let tar = check_output(tar, "packing files")?;
        self.checked(
            "tar -xf - -C \"$HOME\" 2>&1 | grep -v 'Ignoring unknown extended header' >&2; exit 0",
            &[],
            Some(&tar.stdout),
            "copying files",
        )
        .map(drop)
    }

    fn read_home_file(&self, path: &Path) -> Result<Option<Vec<u8>>> {
        ensure_relative(path)?;
        let p = path.to_string_lossy();
        let out = self.sh(
            "[ -f \"$HOME/$1\" ] || exit 3; cat \"$HOME/$1\"",
            &[&p],
            None,
        )?;
        file_contents(out, &format!("reading {p} on {}", self.name()))
    }

    fn write_home_file(&self, path: &Path, data: &[u8]) -> Result<()> {
        ensure_relative(path)?;
        let p = path.to_string_lossy();
        self.checked(
            "mkdir -p \"$(dirname \"$HOME/$1\")\" && cat > \"$HOME/$1\"",
            &[&p],
            Some(data),
            &format!("writing {p}"),
        )
        .map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_for_the_remote_shell() {
        let cmd = remote_sh("printf '%s|' \"$@\"", &["a b", "it's", "$HOME"]);
        let out = Command::new("/bin/sh").args(["-c", &cmd]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "a b|it's|$HOME|");
    }

    #[test]
    fn env_file_round_trips() {
        let env = vec![
            ("A".to_string(), "x'y $z".to_string()),
            ("bad name".into(), "v".into()),
        ];
        let script = format!("{}printf %s \"$A\"", env_file(&env));
        let out = Command::new("/bin/sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "x'y $z");
        assert!(!env_file(&env).contains("bad name"));
    }

    #[test]
    fn parses_tmux_listing() {
        let text = "worki-abc\t/w/feat\tClaude Code\tclaude\nother\t\t\t\nworki-def\t/w/x\t\t\nworki-ghi\t\t\t\n";
        let s = parse_tmux_sessions(text);
        assert_eq!(s.len(), 2);
        assert_eq!(
            s[0],
            RemoteSession {
                id: "abc".into(),
                worktree: "/w/feat".into(),
                title: "Claude Code".into(),
                profile_id: Some("claude".into())
            }
        );
        assert_eq!(
            (s[1].title.as_str(), s[1].profile_id.as_deref()),
            ("Session", None)
        );
    }

    #[test]
    fn names_and_validates_targets() {
        let t = SshTarget::new("devbox", Some("me".into()), Some(2222));
        assert_eq!(t.display(), "me@devbox:2222");
        assert!(t.validate().is_ok());
        assert_eq!(
            SshTarget::new("devbox", Some(" ".into()), None).display(),
            "devbox"
        );
        assert!(SshTarget::new("devbox", Some("-oX".into()), None)
            .validate()
            .is_err());
        assert!(SshTarget::new("devbox", None, Some(0)).validate().is_err());
    }

    #[test]
    fn validates_hosts() {
        for ok in ["devbox", "me@build-01.example.com", "my_alias"] {
            assert!(valid_host(ok), "{ok}");
        }
        for bad in ["", "-oProxyCommand=x", "a b", "h;rm", "h:22"] {
            assert!(!valid_host(bad), "{bad}");
        }
    }
}
