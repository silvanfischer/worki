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
        .file_diff(&created.worktree.path, "README.md")
        .await
        .unwrap();
    assert!(
        diff.contains("-hello") && diff.contains("+changed") && diff.contains("+more"),
        "{diff}"
    );
    let untracked = svc
        .file_diff(&created.worktree.path, ".setup-done")
        .await
        .unwrap();
    assert!(untracked.contains("new file mode"), "{untracked}");
    assert!(svc
        .file_diff(&created.worktree.path, "../app/README.md")
        .await
        .is_err());

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
