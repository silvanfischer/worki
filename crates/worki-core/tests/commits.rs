use std::path::Path;
use std::process::Command;

use worki_core::git;
use worki_core::{BaseUpdate, Host};

fn sh(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sh(dir, &["init", "-q", "-b", "main"]);
    sh(dir, &["config", "user.name", "t"]);
    sh(dir, &["config", "user.email", "t@t"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    sh(dir, &["add", "."]);
    sh(dir, &["commit", "-q", "-m", "init"]);
    sh(dir, &["checkout", "-q", "-b", "work"]);
    tmp
}

#[tokio::test]
async fn commits_only_the_picked_files() {
    let tmp = repo();
    let dir = tmp.path();
    let host = Host::Local;
    std::fs::write(dir.join("a.txt"), "two\n").unwrap();
    std::fs::write(dir.join("new.txt"), "new\n").unwrap();
    std::fs::write(dir.join("keep.txt"), "later\n").unwrap();

    git::commit(
        &host,
        dir,
        &["a.txt".into(), "new.txt".into()],
        "Change a, add new",
    )
    .await
    .unwrap();

    let log = git::branch_log(&host, dir, Some("main")).await.unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].subject, "Change a, add new");
    let recent = git::branch_log(&host, dir, None).await.unwrap();
    assert_eq!(
        recent
            .iter()
            .map(|c| c.subject.as_str())
            .collect::<Vec<_>>(),
        ["Change a, add new", "init"]
    );
    let left = host.git(dir, ["status", "--porcelain"]).await.unwrap();
    assert_eq!(left.trim(), "?? keep.txt");
    assert!(git::commit(&host, dir, &[], "nothing").await.is_err());
    assert_eq!(git::compare_base(&host, dir, "main").await, "main");
    sh(dir, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    assert_eq!(git::compare_base(&host, dir, "main").await, "origin/main");
}

#[tokio::test]
async fn update_from_base_rebases_or_reports_conflicts() {
    let tmp = repo();
    let dir = tmp.path();
    let host = Host::Local;
    assert_eq!(
        git::update_from_base(&host, dir, "main").await.unwrap(),
        BaseUpdate::UpToDate
    );

    std::fs::write(dir.join("b.txt"), "mine\n").unwrap();
    git::commit(&host, dir, &["b.txt".into()], "Add b")
        .await
        .unwrap();
    sh(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("c.txt"), "theirs\n").unwrap();
    sh(dir, &["add", "c.txt"]);
    sh(dir, &["commit", "-q", "-m", "Add c"]);
    sh(dir, &["checkout", "-q", "work"]);
    assert_eq!(
        git::update_from_base(&host, dir, "main").await.unwrap(),
        BaseUpdate::Updated
    );
    assert!(dir.join("c.txt").exists());

    sh(dir, &["checkout", "-q", "main"]);
    std::fs::write(dir.join("a.txt"), "main\n").unwrap();
    sh(dir, &["commit", "-q", "-am", "main edits a"]);
    sh(dir, &["checkout", "-q", "work"]);
    std::fs::write(dir.join("a.txt"), "work\n").unwrap();
    git::commit(&host, dir, &["a.txt".into()], "work edits a")
        .await
        .unwrap();
    let before = host.git(dir, ["rev-parse", "HEAD"]).await.unwrap();
    assert_eq!(
        git::update_from_base(&host, dir, "main").await.unwrap(),
        BaseUpdate::Conflicts {
            files: vec!["a.txt".into()]
        }
    );
    assert_eq!(
        host.git(dir, ["rev-parse", "HEAD"]).await.unwrap(),
        before,
        "rebase undone"
    );
}
