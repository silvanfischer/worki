//! End-to-end checks against real container engines. Each test creates and removes one `worki-*`
//! environment. Run with: `cargo test -p worki-isolation --test containers -- --ignored`
//! (or name one: `… -- --ignored podman`).

use std::path::Path;
use std::process::Command;

use worki_isolation::{Container, Isolation, LaunchRequest, Workspace};

/// A public image with bash, git and tar, so Docker and Podman tests don't need worki's image.
const TEST_IMAGE: &str = "docker.io/library/buildpack-deps:bookworm-scm";

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn run(launch: worki_isolation::Launch) -> (bool, String) {
    let mut cmd = Command::new(&launch.program);
    cmd.args(&launch.args)
        .current_dir(&launch.cwd)
        .envs(launch.env);
    let out = cmd.output().unwrap();
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// Worktrees must live where the engine shares host folders at the same path: Docker Desktop
/// and Podman share `/Users`, so the test repo goes under the home folder.
fn lifecycle(isolation: Isolation) {
    let home = std::env::var("HOME").unwrap();
    let base = Path::new(&home).join("worki");
    std::fs::create_dir_all(&base).unwrap();
    let tmp = tempfile::Builder::new()
        .prefix(".e2e-")
        .tempdir_in(&base)
        .unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let repo = root.join("repo");
    let wt = root.join("wt-e2e");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "."]);
    git(
        &repo,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "init",
        ],
    );
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature",
            wt.to_str().unwrap(),
            "main",
        ],
    );

    let backend = isolation.backend();
    backend.ready().expect("engine available and running");
    let ws = Workspace {
        repo: repo.clone(),
        worktree_root: root.clone(),
        worktree: Some(wt.clone()),
    };
    backend.grant_access(&ws).expect("environment created");
    backend.grant_access(&ws).expect("second grant is a no-op");

    let req = |command: &str| LaunchRequest {
        cwd: wt.clone(),
        command: Some(command.into()),
        interactive: false,
        env: vec![("WORKI_PROBE".into(), "probe-value".into())],
        session: None,
    };
    let (ok, out) = run(backend.launch(&req("echo \"$WORKI_PROBE\"; pwd")).unwrap());
    assert!(
        ok && out.contains("probe-value") && out.contains(wt.to_str().unwrap()),
        "{out}"
    );

    let (ok, out) = run(backend
        .launch(&req(
            "git config --global --add safe.directory '*'; echo x > from-env.txt && git add from-env.txt && git -c user.name=s -c user.email=s@s commit -qm env",
        ))
        .unwrap());
    assert!(ok, "commit failed: {out}");
    assert!(git(&repo, &["log", "--oneline", "-1", "feature"]).contains("env"));

    let (ok, out) = run(backend
        .launch(&req(&format!("touch {}/.git/hooks/evil", repo.display())))
        .unwrap());
    assert!(!ok, "hooks must be read-only: {out}");
    assert!(!repo.join(".git/hooks/evil").exists());

    assert!(backend.instance_exists(&ws));
    let instance = backend.instance_for(&ws).expect("bound instance");
    assert!(instance.which("git").is_some_and(|p| p.starts_with('/')));
    assert!(instance.which("definitely-not-installed").is_none());
    instance
        .write_home_file(Path::new(".worki-test/note.txt"), b"hello")
        .unwrap();
    assert_eq!(
        instance
            .read_home_file(Path::new(".worki-test/note.txt"))
            .unwrap()
            .as_deref(),
        Some(&b"hello"[..])
    );
    assert_eq!(
        instance
            .read_home_file(Path::new(".worki-test/missing"))
            .unwrap(),
        None
    );
    let host_home = root.join("fake-home");
    std::fs::create_dir_all(host_home.join(".claude/agents")).unwrap();
    std::fs::write(host_home.join(".claude/agents/reviewer.md"), "be thorough").unwrap();
    instance
        .copy_home_files(&host_home, &[".claude/agents".into()])
        .unwrap();
    assert_eq!(
        instance
            .read_home_file(Path::new(".claude/agents/reviewer.md"))
            .unwrap()
            .as_deref(),
        Some(&b"be thorough"[..])
    );

    backend.shutdown().expect("stopped");
    backend.grant_access(&ws).expect("restarted");
    let (ok, out) = run(backend.launch(&req("true")).unwrap());
    assert!(ok, "exec after restart: {out}");

    backend.release(&ws).expect("environment removed");
    assert!(!backend.instance_exists(&ws));

    backend.grant_access(&ws).expect("environment recreated");
    assert_eq!(Container::release_everywhere(&ws), Vec::<String>::new());
    assert!(
        !backend.instance_exists(&ws),
        "the sweep must remove the worktree's environment"
    );

    backend.grant_access(&ws).expect("environment recreated");
    assert!(backend.instance_exists(&ws));
    backend
        .forget_project(&Workspace {
            worktree: None,
            ..ws.clone()
        })
        .expect("project forgotten");
    assert!(
        !backend.instance_exists(&ws),
        "project removal must delete the worktree's environment"
    );
    git(
        &repo,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
}

fn pull(engine: &str) {
    let ok = Command::new(engine)
        .args(["pull", "-q", TEST_IMAGE])
        .output()
        .is_ok_and(|o| o.status.success());
    assert!(ok, "{engine} pull {TEST_IMAGE} failed");
}

#[test]
#[ignore = "needs Docker Sandboxes (sbx)"]
fn sbx_lifecycle() {
    lifecycle(Isolation::DockerSandbox {
        image: "shell".into(),
        template: None,
    });
}

#[test]
#[ignore = "needs Docker Desktop running"]
fn docker_lifecycle() {
    pull("docker");
    lifecycle(Isolation::Docker {
        image: TEST_IMAGE.into(),
    });
}

#[test]
#[ignore = "needs a running Podman machine"]
fn podman_lifecycle() {
    pull("podman");
    lifecycle(Isolation::Podman {
        image: TEST_IMAGE.into(),
    });
}
