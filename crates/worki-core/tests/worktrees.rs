use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

use worki_core::store::Store;
use worki_core::*;

fn sh(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn init_repo(dir: &Path) {
    sh(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("README.md"), "hello\n").unwrap();
    std::fs::write(
        dir.join(".worki.toml"),
        "[worktree]\nsetup = \"echo setup-ran-on-$WORKI_BRANCH && touch .setup-done\"\n",
    )
    .unwrap();
    sh(dir, &["add", "."]);
    sh(dir, &["commit", "-q", "-m", "init"]);
}

#[tokio::test]
async fn project_and_worktree_lifecycle() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("app");
    std::fs::create_dir(&repo).unwrap();
    init_repo(&repo);

    let svc = ProjectService::new(Store::in_memory().unwrap());
    let project = svc.add_project(&repo).await.unwrap();
    assert_eq!(project.name, "app");
    assert_eq!(project.default_base, "main");
    assert!(matches!(
        svc.add_project(&repo).await,
        Err(Error::ProjectExists(_))
    ));

    let root = tmp.path().join("worktrees");
    let project = svc
        .update_project(
            &project.id,
            ProjectPatch {
                worktree_root: Some(root.clone()),
                ..Default::default()
            },
        )
        .unwrap();

    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let created = svc
        .create_worktree(
            &project.id,
            CreateWorktree {
                name: "feature/Login".into(),
                base: "main".into(),
                run_setup: true,
                isolation: Default::default(),
            },
            move |l| sink.lock().unwrap().push(l),
        )
        .await
        .unwrap();
    assert_eq!(created.setup_error, None);
    assert_eq!(created.worktree.branch.as_deref(), Some("feature/Login"));
    assert!(created.worktree.path.ends_with("feature-login"));
    assert!(created.worktree.path.join(".setup-done").exists());
    assert!(lines
        .lock()
        .unwrap()
        .iter()
        .any(|l| matches!(l, LogLine::Stdout(s) if s == "setup-ran-on-feature/Login")));

    let wts = svc.list_worktrees(&project.id).await.unwrap();
    assert_eq!(wts.len(), 2);
    assert_eq!(wts[0].base, None);
    assert_eq!(wts[1].base.as_deref(), Some("main"));

    let wt_path = created.worktree.path.clone();
    let main_path = wts[0].path.clone();
    assert_eq!(
        svc.isolation_for_worktree(&project, &wt_path).unwrap(),
        Isolation::None
    );
    let agent_user = ProjectIsolation::MacUser {
        user: "_worki_test".into(),
    };
    let effective = svc
        .set_worktree_isolation(&project.id, &wt_path, agent_user.clone())
        .await
        .unwrap();
    assert_eq!(
        effective,
        Isolation::MacUser {
            user: "_worki_test".into()
        }
    );
    assert_eq!(svc.worktree_isolation_choice(&wt_path).unwrap(), agent_user);
    assert_eq!(
        svc.isolation_for_worktree(&project, &main_path).unwrap(),
        Isolation::None,
        "other worktrees keep the default"
    );
    assert!(svc
        .isolations_in_use()
        .unwrap()
        .contains(&Isolation::MacUser {
            user: "_worki_test".into()
        }));

    let dup = CreateWorktree {
        name: "feature/Login".into(),
        base: "main".into(),
        run_setup: false,
        isolation: Default::default(),
    };
    assert!(matches!(
        svc.create_worktree(&project.id, dup, |_| {}).await,
        Err(Error::BranchExists(_))
    ));
    let bad_base = CreateWorktree {
        name: "other".into(),
        base: "nope".into(),
        run_setup: false,
        isolation: Default::default(),
    };
    assert!(matches!(
        svc.create_worktree(&project.id, bad_base, |_| {}).await,
        Err(Error::UnknownRef(_))
    ));
    assert!(wts[0].is_main);

    std::fs::write(created.worktree.path.join("README.md"), "changed\nmore\n").unwrap();
    let status = svc.worktree_status(&created.worktree.path).await.unwrap();
    let codes: Vec<_> = status
        .changed_files
        .iter()
        .map(|f| (f.code.as_str(), f.path.as_str()))
        .collect();
    assert!(codes.contains(&(" M", "README.md")));
    assert!(codes.contains(&("??", ".setup-done")));
    assert_eq!((status.insertions, status.deletions), (2, 1));

    let diff = svc
        .file_diff(&created.worktree.path, "README.md", None)
        .await
        .unwrap();
    assert!(
        diff.contains("-hello") && diff.contains("+changed") && diff.contains("+more"),
        "{diff}"
    );
    let untracked = svc
        .file_diff(&created.worktree.path, ".setup-done", None)
        .await
        .unwrap();
    assert!(untracked.contains("new file mode"), "{untracked}");
    assert!(svc
        .file_diff(&created.worktree.path, "../app/README.md", None)
        .await
        .is_err());

    std::fs::write(created.worktree.path.join("lib.rs"), "fn a() {}\n").unwrap();
    sh(&created.worktree.path, &["add", "lib.rs"]);
    sh(&created.worktree.path, &["commit", "-q", "-m", "lib"]);
    let branch = svc
        .branch_changes(&created.worktree.path, "main")
        .await
        .unwrap();
    let codes: Vec<_> = branch
        .files
        .iter()
        .map(|f| (f.code.as_str(), f.path.as_str()))
        .collect();
    assert_eq!(branch.commits, 1);
    assert_eq!(branch.behind, 0);
    assert!(codes.contains(&("A ", "lib.rs")), "{codes:?}");
    assert!(codes.contains(&("M ", "README.md")), "{codes:?}");
    assert!(codes.contains(&("??", ".setup-done")), "{codes:?}");
    let committed = svc
        .file_diff(&created.worktree.path, "lib.rs", Some("main"))
        .await
        .unwrap();
    assert!(committed.contains("+fn a() {}"), "{committed}");
    assert!(svc
        .file_diff(&created.worktree.path, "lib.rs", None)
        .await
        .unwrap()
        .is_empty());

    let dirty = RemoveWorktree {
        path: created.worktree.path.clone(),
        force: false,
        delete_branch: true,
    };
    assert!(svc.remove_worktree(&project.id, dirty).await.is_err());
    let forced = RemoveWorktree {
        path: created.worktree.path.clone(),
        force: true,
        delete_branch: true,
    };
    svc.remove_worktree(&project.id, forced).await.unwrap();
    assert_eq!(
        svc.worktree_isolation_choice(&wt_path).unwrap(),
        ProjectIsolation::Inherit,
        "choice removed with the worktree"
    );
    assert_eq!(svc.list_worktrees(&project.id).await.unwrap().len(), 1);
    assert!(!svc
        .list_branches(&project.id)
        .await
        .unwrap()
        .contains(&"feature/Login".to_string()));

    let main = RemoveWorktree {
        path: repo.clone(),
        force: true,
        delete_branch: false,
    };
    assert!(svc.remove_worktree(&project.id, main).await.is_err());
}

#[tokio::test]
async fn rejects_non_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = ProjectService::new(Store::in_memory().unwrap());
    assert!(matches!(
        svc.add_project(tmp.path()).await,
        Err(Error::NotARepo(_))
    ));
}

#[test]
fn saved_ssh_hosts_follow_into_projects() {
    let store = Store::in_memory().unwrap();
    let remote_project = |id: &str, host_id: Option<&str>| Project {
        id: id.into(),
        name: id.into(),
        repo_path: "/src/app".into(),
        worktree_root: "/src/wt".into(),
        default_base: "main".into(),
        isolation: Default::default(),
        setup_script: None,
        remote: Some(Remote {
            host: "old".into(),
            user: None,
            port: None,
            keep_alive: true,
            host_id: host_id.map(Into::into),
        }),
    };
    store
        .insert_project(&remote_project("linked", Some("h1")))
        .unwrap();
    store.insert_project(&remote_project("own", None)).unwrap();
    let svc = ProjectService::new(store);
    let saved = svc
        .save_ssh_host(SshHost {
            id: "h1".into(),
            name: " ".into(),
            host: "devbox".into(),
            user: Some("".into()),
            port: None,
        })
        .unwrap();
    assert_eq!(
        (saved.name.as_str(), saved.user.as_deref()),
        ("devbox", None)
    );
    svc.save_ssh_host(SshHost {
        port: Some(2222),
        user: Some("me".into()),
        ..saved.clone()
    })
    .unwrap();
    let linked = svc.project("linked").unwrap().remote.unwrap();
    assert_eq!(
        (linked.host.as_str(), linked.user.as_deref(), linked.port),
        ("devbox", Some("me"), Some(2222))
    );
    assert_eq!(svc.project("own").unwrap().remote.unwrap().host, "old");

    svc.delete_ssh_host(&saved.id).unwrap();
    assert!(svc.ssh_hosts().unwrap().is_empty());
    let kept = svc.project("linked").unwrap().remote.unwrap();
    assert_eq!((kept.host.as_str(), kept.host_id), ("devbox", None));
}

#[tokio::test]
async fn lists_newest_worktree_first_after_the_main_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("app");
    std::fs::create_dir(&repo).unwrap();
    init_repo(&repo);
    for (name, stamp) in [
        ("older", "202601010000"),
        ("newer", "202602010000"),
        ("oldest", "202501010000"),
    ] {
        sh(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                name,
                &tmp.path().join(name).to_string_lossy(),
            ],
        );
        let status = Command::new("touch")
            .args(["-t", stamp])
            .arg(repo.join(".git/worktrees").join(name).join("commondir"))
            .status()
            .unwrap();
        assert!(status.success());
    }

    let svc = ProjectService::new(Store::in_memory().unwrap());
    let project = svc.add_project(&repo).await.unwrap();
    let branches: Vec<_> = svc
        .list_worktrees(&project.id)
        .await
        .unwrap()
        .into_iter()
        .map(|wt| wt.branch.unwrap())
        .collect();
    assert_eq!(branches, ["main", "newer", "older", "oldest"]);
}

#[tokio::test]
async fn a_worktree_made_again_starts_without_the_old_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("app");
    std::fs::create_dir(&repo).unwrap();
    init_repo(&repo);
    let svc = ProjectService::new(Store::in_memory().unwrap());
    let project = svc.add_project(&repo).await.unwrap();
    let project = svc
        .update_project(
            &project.id,
            ProjectPatch {
                worktree_root: Some(tmp.path().join("worktrees")),
                ..Default::default()
            },
        )
        .unwrap();
    let create = || {
        svc.create_worktree(
            &project.id,
            CreateWorktree {
                name: "fix".into(),
                base: "main".into(),
                run_setup: false,
                isolation: Default::default(),
            },
            |_| {},
        )
    };
    let save = |path: &Path| {
        svc.save_session(SavedSession {
            id: "s1".into(),
            project_id: project.id.clone(),
            worktree_path: path.into(),
            profile_id: "claude".into(),
            chat: true,
        })
        .unwrap()
    };

    let path = create().await.unwrap().worktree.path;
    save(&path);
    svc.remove_worktree(
        &project.id,
        RemoveWorktree {
            path: path.clone(),
            force: true,
            delete_branch: true,
        },
    )
    .await
    .unwrap();
    assert!(svc.saved_sessions().unwrap().is_empty());

    save(&path);
    create().await.unwrap();
    assert!(svc.saved_sessions().unwrap().is_empty());
}
