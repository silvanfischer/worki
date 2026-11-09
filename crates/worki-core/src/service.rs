use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::RepoConfig;
use crate::model::*;
use worki_isolation::{Backend, LaunchRequest, Workspace};

use crate::openers::{self, OpenerSettings};
use crate::setup::run_logged;
use crate::store::Store;
use crate::{git, paths, secrets, Error, Host, Result};

const AGENT_SETTINGS_KEY: &str = "agents";
const ISOLATION_SETTINGS_KEY: &str = "isolation";
const OPENER_SETTINGS_KEY: &str = "openers";
const SSH_HOSTS_KEY: &str = "sshHosts";
/// Per-worktree isolation choices, keyed by worktree path.
const WORKTREE_ISOLATION_KEY: &str = "worktreeIsolation";

type WorktreeChoices = BTreeMap<String, ProjectIsolation>;

pub struct ProjectService {
    store: Store,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedWorktree {
    pub worktree: Worktree,
    pub setup_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupScripts {
    pub project: Option<String>,
    pub repo_file: Option<String>,
}

impl SetupScripts {
    /// The project's own script wins over the shared `.worki.toml`.
    pub fn effective(&self) -> Option<&str> {
        self.project
            .as_deref()
            .or(self.repo_file.as_deref())
            .filter(|s| !s.trim().is_empty())
    }
}

impl ProjectService {
    pub fn new(store: Store) -> Self {
        Self { store }
    }

    pub fn open_default() -> Result<Self> {
        Ok(Self::new(Store::open(&paths::data_dir().join("worki.db"))?))
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        self.store.list_projects()
    }

    pub fn project(&self, id: &str) -> Result<Project> {
        self.store.get_project(id)
    }

    pub async fn add_project(&self, path: &Path) -> Result<Project> {
        self.add_project_on(Host::Local, None, path).await
    }

    /// Adds a repository that lives on `remote.host`; `path` is a path on that host.
    pub async fn add_remote_project(&self, remote: Remote, path: &Path) -> Result<Project> {
        let remote = self.with_saved_host(remote)?;
        remote.target().validate().map_err(Error::isolation)?;
        self.add_project_on(Host::Ssh(remote.target()), Some(remote), path)
            .await
    }

    async fn add_project_on(
        &self,
        host: Host,
        remote: Option<Remote>,
        path: &Path,
    ) -> Result<Project> {
        let path = host.expand_tilde(path).await?;
        let repo = git::main_checkout(&host, &path).await?;
        if self
            .store
            .find_by_repo(
                &repo,
                remote.as_ref().map(|r| r.target().display()).as_deref(),
            )?
            .is_some()
        {
            return Err(Error::ProjectExists(repo));
        }
        let name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "project".into());
        let config_base = RepoConfig::load(&host, &repo)
            .await
            .ok()
            .flatten()
            .and_then(|c| c.worktree.base);
        let default_base = match config_base {
            Some(base) => base,
            None => git::default_base(&host, &repo).await,
        };
        let worktree_root = match &host {
            Host::Local => paths::default_worktree_root(&name),
            Host::Ssh(_) => host
                .home()
                .await?
                .join("worki/worktrees")
                .join(paths::slug(&name)),
        };
        let project = Project {
            id: uuid::Uuid::new_v4().to_string(),
            worktree_root,
            name,
            repo_path: repo,
            default_base,
            isolation: ProjectIsolation::Inherit,
            setup_script: None,
            remote,
        };
        self.store.insert_project(&project)?;
        Ok(project)
    }

    pub fn update_project(&self, id: &str, patch: ProjectPatch) -> Result<Project> {
        let mut p = self.store.get_project(id)?;
        if let Some(name) = patch.name.filter(|n| !n.trim().is_empty()) {
            p.name = name.trim().into();
        }
        if let Some(root) = patch.worktree_root {
            p.worktree_root = if p.remote.is_some() {
                root
            } else {
                paths::expand_tilde(&root)
            };
        }
        if let Some(base) = patch.default_base.filter(|b| !b.trim().is_empty()) {
            p.default_base = base.trim().into();
        }
        if let Some(isolation) = patch.isolation {
            isolation
                .resolve(&Isolation::None)
                .validate()
                .map_err(Error::isolation)?;
            p.isolation = isolation;
        }
        if let Some(script) = patch.setup_script {
            p.setup_script = Some(script).filter(|s| !s.trim().is_empty());
        }
        if let (Some(remote), true) = (patch.remote, p.remote.is_some()) {
            let remote = self.with_saved_host(remote)?;
            remote.target().validate().map_err(Error::isolation)?;
            p.remote = Some(remote);
        }
        self.store.update_project(&p)?;
        Ok(p)
    }

    /// Removes the project from worki and cleans up its isolation (environments, access
    /// grants). When the isolation can't be reached (engine stopped or gone), the project is still
    /// removed and the cleanup is skipped with a warning.
    pub async fn remove_project(&self, id: &str) -> Result<Option<String>> {
        let project = self.store.get_project(id)?;
        let global = self.isolation_settings()?.default;
        let in_project = |path: &str| project.contains(Path::new(path));
        let mut choices = self.worktree_choices()?;
        let mut isolations = vec![self.isolation_for(&project)?];
        let worktree_isolations = choices
            .iter()
            .filter(|(path, choice)| **choice != ProjectIsolation::Inherit && in_project(path))
            .map(|(_, choice)| choice.resolve(&global));
        push_unique(&mut isolations, worktree_isolations);
        let mut warnings = Vec::new();
        for isolation in isolations.into_iter().filter(|i| *i != Isolation::None) {
            let ws = Self::workspace(&project, None);
            let label = isolation.label();
            let skipped = blocking(move || {
                let backend = isolation.backend();
                if let Err(e) = backend.ready() {
                    return Ok(Some(format!("Skipped cleaning up {label}: {e:#}")));
                }
                backend.forget_project(&ws).map(|_| None)
            })
            .await?;
            warnings.extend(skipped);
        }
        choices.retain(|path, _| !in_project(path));
        self.save_worktree_choices(&choices)?;
        self.store.delete_project(id)?;
        Ok((!warnings.is_empty()).then(|| warnings.join("\n")))
    }

    pub fn reorder_projects(&self, ids: &[String]) -> Result<()> {
        self.store.reorder_projects(ids)
    }

    pub fn agent_settings(&self) -> Result<AgentSettings> {
        let mut settings: AgentSettings = self.setting(AGENT_SETTINGS_KEY)?;
        if let Some(names) = settings.secrets.remove(LEGACY_SCOPE) {
            let scope = self.isolation_settings()?.default.scope_key();
            for name in &names {
                secrets::adopt_legacy(&scope, name)?;
                settings.add_secret(&scope, name);
            }
            self.save_agent_settings(&settings)?;
        }
        Ok(settings.with_known_installers())
    }

    /// Installs `command`'s agent in every environment of `isolation`, streaming output, and
    /// remembers it for environments created later.
    pub async fn install_agent(
        &self,
        isolation: &Isolation,
        command: &str,
        log: impl Fn(LogLine) + Send + Sync + Clone + 'static,
    ) -> Result<()> {
        let backend = isolation.backend();
        let instances = blocking(move || backend.instances()).await?;
        if instances.is_empty() {
            log(LogLine::Info(
                "No environments yet; it will be installed in each new one.".into(),
            ));
        }
        for (i, instance) in instances.iter().enumerate() {
            if instances.len() > 1 {
                log(LogLine::Info(format!(
                    "— environment {} of {}",
                    i + 1,
                    instances.len()
                )));
            }
            self.install_in(isolation, instance.as_ref(), command, log.clone())
                .await?;
        }
        self.update_agent_settings(|s| s.remember_setup(&isolation.scope_key(), command, true))
    }

    async fn install_in(
        &self,
        isolation: &Isolation,
        instance: &dyn Backend,
        command: &str,
        log: impl Fn(LogLine) + Send + Sync + Clone + 'static,
    ) -> Result<()> {
        let settings = self.agent_settings()?;
        let script = settings
            .profiles
            .iter()
            .find(|p| p.executable() == Some(command))
            .and_then(|p| p.install.clone())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| Error::NoInstaller(command.into()))?;
        log(LogLine::Info(format!("$ {script}")));
        let req = LaunchRequest {
            cwd: "/tmp".into(),
            command: Some(script),
            interactive: false,
            env: self.secret_env(isolation)?,
            session: None,
        };
        let launch = instance.launch(&req).map_err(Error::isolation)?;
        run_logged(launch, log).await
    }

    pub fn save_agent_settings(&self, settings: &AgentSettings) -> Result<()> {
        self.store.set_setting(AGENT_SETTINGS_KEY, settings)
    }

    pub fn update_agent_settings(&self, change: impl FnOnce(&mut AgentSettings)) -> Result<()> {
        let mut settings = self.agent_settings()?;
        change(&mut settings);
        self.save_agent_settings(&settings)
    }

    fn setting<T: DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
        Ok(self.store.get_setting(key)?.unwrap_or_default())
    }

    /// Apps worktrees can be opened in. Filled with the installed well-known apps the first time.
    pub fn opener_settings(&self) -> Result<OpenerSettings> {
        if let Some(settings) = self.store.get_setting(OPENER_SETTINGS_KEY)? {
            return Ok(settings);
        }
        let apps = openers::detect_installed();
        let default_app = apps.first().map(|a| a.id.clone());
        let settings = OpenerSettings { apps, default_app };
        self.save_opener_settings(&settings)?;
        Ok(settings)
    }

    pub fn save_opener_settings(&self, settings: &OpenerSettings) -> Result<()> {
        self.store.set_setting(OPENER_SETTINGS_KEY, settings)
    }

    pub fn ssh_hosts(&self) -> Result<Vec<SshHost>> {
        self.setting(SSH_HOSTS_KEY)
    }

    pub fn ssh_host(&self, id: &str) -> Result<SshHost> {
        self.ssh_hosts()?
            .into_iter()
            .find(|h| h.id == id)
            .ok_or_else(|| Error::SshHostNotFound(id.into()))
    }

    /// Adds or replaces a saved host (a new one when `id` is empty) and updates the projects
    /// connecting through it.
    pub fn save_ssh_host(&self, mut host: SshHost) -> Result<SshHost> {
        host.name = host.name.trim().into();
        host.host = host.host.trim().into();
        host.user = host.user.filter(|u| !u.trim().is_empty());
        if host.name.is_empty() {
            host.name = host.host.clone();
        }
        host.target().validate().map_err(Error::isolation)?;
        if host.id.is_empty() {
            host.id = uuid::Uuid::new_v4().to_string();
        }
        let mut hosts = self.ssh_hosts()?;
        match hosts.iter_mut().find(|h| h.id == host.id) {
            Some(existing) => *existing = host.clone(),
            None => hosts.push(host.clone()),
        }
        self.store.set_setting(SSH_HOSTS_KEY, &hosts)?;
        for mut p in self.store.list_projects()? {
            if let Some(remote) = p
                .remote
                .as_mut()
                .filter(|r| r.host_id.as_deref() == Some(&host.id))
            {
                host.apply(remote);
                self.store.update_project(&p)?;
            }
        }
        Ok(host)
    }

    /// Removes a saved host. Projects using it keep its connection settings as their own.
    pub fn delete_ssh_host(&self, id: &str) -> Result<()> {
        let mut hosts = self.ssh_hosts()?;
        hosts.retain(|h| h.id != id);
        self.store.set_setting(SSH_HOSTS_KEY, &hosts)?;
        for mut p in self.store.list_projects()? {
            if let Some(remote) = p
                .remote
                .as_mut()
                .filter(|r| r.host_id.as_deref() == Some(id))
            {
                remote.host_id = None;
                self.store.update_project(&p)?;
            }
        }
        Ok(())
    }

    fn with_saved_host(&self, mut remote: Remote) -> Result<Remote> {
        if let Some(id) = remote.host_id.clone() {
            self.ssh_host(&id)?.apply(&mut remote);
        }
        Ok(remote)
    }

    pub fn isolation_settings(&self) -> Result<IsolationSettings> {
        self.setting(ISOLATION_SETTINGS_KEY)
    }

    pub fn save_isolation_settings(&self, settings: &IsolationSettings) -> Result<()> {
        self.store.set_setting(ISOLATION_SETTINGS_KEY, settings)
    }

    pub fn isolation_for(&self, project: &Project) -> Result<Isolation> {
        if let Some(remote) = &project.remote {
            return Ok(Isolation::Ssh {
                host: remote.host.clone(),
                user: remote.user.clone(),
                port: remote.port,
                keep_alive: remote.keep_alive,
            });
        }
        Ok(project
            .isolation
            .resolve(&self.isolation_settings()?.default))
    }

    fn worktree_choices(&self) -> Result<WorktreeChoices> {
        self.setting(WORKTREE_ISOLATION_KEY)
    }

    fn save_worktree_choices(&self, choices: &WorktreeChoices) -> Result<()> {
        self.store.set_setting(WORKTREE_ISOLATION_KEY, choices)
    }

    /// Stores the worktree's own choice; `Inherit` forgets it.
    fn set_worktree_choice(&self, worktree: &Path, choice: ProjectIsolation) -> Result<()> {
        let mut choices = self.worktree_choices()?;
        let key = path_key(worktree);
        let changed = if choice == ProjectIsolation::Inherit {
            choices.remove(&key).is_some()
        } else {
            choices.insert(key, choice);
            true
        };
        if changed {
            self.save_worktree_choices(&choices)?;
        }
        Ok(())
    }

    /// The worktree's own choice; `Inherit` means "project default".
    pub fn worktree_isolation_choice(&self, worktree: &Path) -> Result<ProjectIsolation> {
        Ok(self
            .worktree_choices()?
            .remove(&path_key(worktree))
            .unwrap_or_default())
    }

    /// Isolation a worktree runs in: its own choice, else the project default, else the global
    /// default. Remote projects always use their SSH connection.
    pub fn isolation_for_worktree(&self, project: &Project, worktree: &Path) -> Result<Isolation> {
        if project.remote.is_some() {
            return self.isolation_for(project);
        }
        match self.worktree_isolation_choice(worktree)? {
            ProjectIsolation::Inherit => self.isolation_for(project),
            choice => Ok(choice.resolve(&self.isolation_settings()?.default)),
        }
    }

    /// Changes a worktree's isolation. The environment of the old isolation (e.g. its container)
    /// is removed; the next session sets up the new one.
    pub async fn set_worktree_isolation(
        &self,
        project_id: &str,
        worktree: &Path,
        choice: ProjectIsolation,
    ) -> Result<Isolation> {
        let project = self.store.get_project(project_id)?;
        choice
            .resolve(&Isolation::None)
            .validate()
            .map_err(Error::isolation)?;
        let old = self.isolation_for_worktree(&project, worktree)?;
        self.set_worktree_choice(worktree, choice)?;
        let new = self.isolation_for_worktree(&project, worktree)?;
        if old != new && old != Isolation::None {
            let ws = Self::workspace(&project, Some(worktree));
            let released = blocking(move || {
                let backend = old.backend();
                backend.ready()?;
                backend.release(&ws)
            })
            .await;
            if let Err(e) = released {
                tracing::warn!(
                    "removing the old environment of {}: {e}",
                    worktree.display()
                );
            }
        }
        Ok(new)
    }

    /// The host a worktree or repository path belongs to, from the projects that contain it.
    pub fn host_for_path(&self, path: &Path) -> Result<Host> {
        Ok(self
            .list_projects()?
            .iter()
            .filter(|p| p.remote.is_some())
            .find(|p| p.contains(path))
            .map(Host::of)
            .unwrap_or(Host::Local))
    }

    pub fn workspace(project: &Project, worktree: Option<&Path>) -> Workspace {
        Workspace {
            repo: project.repo_path.clone(),
            worktree_root: project.worktree_root.clone(),
            worktree: worktree.map(Path::to_path_buf),
        }
    }

    /// Gives the project's isolated environment access to `worktree` (and the git data it
    /// needs), nothing else. Blocking work runs off the async runtime.
    pub async fn grant_access(
        &self,
        project: &Project,
        worktree: Option<&Path>,
    ) -> Result<Isolation> {
        let isolation = match worktree {
            Some(w) => self.isolation_for_worktree(project, w)?,
            None => self.isolation_for(project)?,
        };
        if isolation != Isolation::None {
            let backend = isolation.backend();
            let ws = Self::workspace(project, worktree);
            let created = blocking(move || {
                let existed = backend.instance_exists(&ws);
                backend.grant_access(&ws)?;
                Ok((!existed).then(|| backend.instance_for(&ws)).flatten())
            })
            .await?;
            if let Some(instance) = created {
                self.set_up_new_instance(&isolation, instance).await;
            }
        }
        Ok(isolation)
    }

    /// Repeats the remembered agent setup (installs, config copies) in a just-created environment.
    /// Failures are logged, not fatal: the session can still start and the user can retry.
    async fn set_up_new_instance(&self, isolation: &Isolation, instance: Box<dyn Backend>) {
        let Ok(settings) = self.agent_settings() else {
            return;
        };
        let setup = settings
            .environment_setup
            .get(&isolation.scope_key())
            .cloned()
            .unwrap_or_default();
        let instance: Arc<dyn Backend> = instance.into();
        for executable in &setup.install {
            if let Err(e) = self
                .install_in(isolation, instance.as_ref(), executable, |_| {})
                .await
            {
                tracing::warn!("installing {executable} in a new environment: {e}");
            }
        }
        for executable in setup.copy_config {
            let Some(kit) = crate::agents::kit(&executable) else {
                continue;
            };
            let instance = instance.clone();
            if let Err(e) =
                blocking(move || crate::agents::copy_config(instance.as_ref(), kit)).await
            {
                tracing::warn!("copying {executable} config into a new environment: {e}");
            }
        }
    }

    /// Every distinct isolation in use: the global default, project defaults and worktree choices.
    pub fn isolations_in_use(&self) -> Result<Vec<Isolation>> {
        let global = self.isolation_settings()?.default;
        let mut all = vec![global.clone()];
        let project_defaults = self
            .list_projects()?
            .iter()
            .map(|p| self.isolation_for(p))
            .collect::<Result<Vec<_>>>()?;
        push_unique(&mut all, project_defaults);
        let choices = self.worktree_choices()?;
        push_unique(
            &mut all,
            choices
                .values()
                .filter(|c| **c != ProjectIsolation::Inherit)
                .map(|c| c.resolve(&global)),
        );
        Ok(all)
    }

    /// Secret values configured for `isolation` that have one stored.
    pub fn secret_env(&self, isolation: &Isolation) -> Result<Vec<(String, String)>> {
        let scope = isolation.scope_key();
        Ok(secrets::env(
            &scope,
            self.agent_settings()?.secret_names(&scope),
        ))
    }

    pub async fn setup_scripts(&self, id: &str) -> Result<SetupScripts> {
        let p = self.store.get_project(id)?;
        Ok(SetupScripts {
            repo_file: RepoConfig::load(&Host::of(&p), &p.repo_path)
                .await?
                .and_then(|c| c.worktree.setup),
            project: p.setup_script,
        })
    }

    pub async fn list_worktrees(&self, id: &str) -> Result<Vec<Worktree>> {
        let p = self.store.get_project(id)?;
        git::list_worktrees(&Host::of(&p), &p.repo_path).await
    }

    pub async fn list_branches(&self, id: &str) -> Result<Vec<String>> {
        let p = self.store.get_project(id)?;
        git::list_branches(&Host::of(&p), &p.repo_path).await
    }

    pub async fn worktree_status(&self, path: &Path) -> Result<WorktreeStatus> {
        git::status(&self.host_for_path(path)?, path).await
    }

    pub async fn file_diff(
        &self,
        worktree: &Path,
        file: &str,
        base: Option<&str>,
    ) -> Result<String> {
        git::file_diff(&self.host_for_path(worktree)?, worktree, file, base).await
    }

    pub async fn branch_changes(&self, worktree: &Path, base: &str) -> Result<BranchChanges> {
        git::branch_changes(&self.host_for_path(worktree)?, worktree, base).await
    }

    pub async fn create_worktree(
        &self,
        id: &str,
        req: CreateWorktree,
        log: impl Fn(LogLine) + Send + Sync + Clone + 'static,
    ) -> Result<CreatedWorktree> {
        let project = self.store.get_project(id)?;
        let host = &Host::of(&project);
        let repo = &project.repo_path;
        let branch = &paths::branch_name(&req.name);
        let base = req.base.trim();
        git::validate_branch_name(host, repo, branch).await?;
        if git::branch_exists(host, repo, branch).await {
            return Err(Error::BranchExists(branch.into()));
        }
        if !git::ref_exists(host, repo, base).await {
            return Err(Error::UnknownRef(base.into()));
        }

        let path = paths::worktree_path(&project.worktree_root, branch);
        if host.exists(&path).await? {
            return Err(Error::PathExists(path));
        }
        host.create_dir_all(&project.worktree_root).await?;

        log(LogLine::Info(format!(
            "Creating worktree {} on new branch {branch} from {base}",
            path.display()
        )));
        git::add_worktree(host, repo, &path, branch, base).await?;

        let worktree = find_worktree(host, repo, &path).await?;
        if req.isolation != ProjectIsolation::Inherit {
            self.set_worktree_choice(&worktree.path, req.isolation)?;
        }
        let isolation = self.isolation_for_worktree(&project, &worktree.path)?;
        if isolation != Isolation::None {
            log(LogLine::Info(format!(
                "Giving {} access to this worktree…",
                isolation.label()
            )));
            self.grant_access(&project, Some(&worktree.path)).await?;
        }
        let mut setup_error = None;
        if req.run_setup {
            if let Some(script) = self.setup_scripts(id).await?.effective() {
                log(LogLine::Info(format!(
                    "Running setup script as {}…",
                    isolation.label()
                )));
                let mut env = vec![
                    ("WORKI_ROOT_PATH".to_string(), path_key(repo)),
                    ("WORKI_WORKTREE_PATH".to_string(), path_key(&worktree.path)),
                    ("WORKI_BRANCH".to_string(), branch.to_string()),
                ];
                env.extend(self.secret_env(&isolation)?);
                if let Err(e) =
                    run_setup_script(&isolation, &worktree.path, script, env, log.clone()).await
                {
                    log(LogLine::Stderr(e.to_string()));
                    setup_error = Some(e.to_string());
                }
            }
        }
        log(LogLine::Info("Done.".into()));
        Ok(CreatedWorktree {
            worktree,
            setup_error,
        })
    }

    pub async fn remove_worktree(&self, id: &str, req: RemoveWorktree) -> Result<()> {
        let project = self.store.get_project(id)?;
        let host = &Host::of(&project);
        let repo = &project.repo_path;
        let wt = find_worktree(host, repo, &req.path).await?;
        if wt.is_main {
            return Err(Error::Git {
                args: "worktree remove".into(),
                stderr: "the main checkout cannot be removed".into(),
            });
        }
        let isolation = self.isolation_for_worktree(&project, &wt.path)?;
        let ws = Self::workspace(&project, Some(&wt.path));
        let local = !host.is_remote();
        blocking(move || {
            if isolation != Isolation::None {
                isolation.backend().release(&ws)?;
            }
            if local {
                for warning in worki_isolation::Container::release_everywhere(&ws) {
                    tracing::warn!(
                        "cleaning up {}: {warning}",
                        ws.worktree.as_deref().unwrap_or(&ws.repo).display()
                    );
                }
            }
            Ok(())
        })
        .await?;
        git::remove_worktree(host, repo, &wt.path, req.force).await?;
        self.set_worktree_choice(&wt.path, ProjectIsolation::Inherit)?;
        if let (true, Some(branch)) = (req.delete_branch, wt.branch.as_deref()) {
            git::delete_branch(host, repo, branch, req.force).await?;
        }
        Ok(())
    }
}

async fn find_worktree(host: &Host, repo: &Path, path: &Path) -> Result<Worktree> {
    let wanted = host.canonical(path).await;
    let mut found = None;
    for w in git::list_worktrees(host, repo).await? {
        if w.path == wanted
            || w.path == path
            || (!host.is_remote() && host.canonical(&w.path).await == wanted)
        {
            found = Some(w);
            break;
        }
    }
    found.ok_or_else(|| Error::Git {
        args: "worktree list".into(),
        stderr: format!("no worktree at {}", path.display()),
    })
}

async fn run_setup_script(
    isolation: &Isolation,
    worktree: &Path,
    script: &str,
    env: Vec<(String, String)>,
    log: impl Fn(LogLine) + Send + Sync + Clone + 'static,
) -> Result<()> {
    let req = LaunchRequest {
        cwd: worktree.to_path_buf(),
        command: Some(script.to_owned()),
        interactive: false,
        env,
        session: None,
    };
    let isolation = isolation.clone();
    let launch = blocking(move || isolation.backend().launch(&req)).await?;
    run_logged(launch, log).await
}

/// Runs blocking isolation work (backend calls may shell out or talk SSH) off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Isolation(e.to_string()))?
        .map_err(Error::isolation)
}

fn push_unique(all: &mut Vec<Isolation>, more: impl IntoIterator<Item = Isolation>) {
    for i in more {
        if !all.contains(&i) {
            all.push(i);
        }
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
