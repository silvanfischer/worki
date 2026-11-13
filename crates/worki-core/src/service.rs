use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::RepoConfig;
use crate::model::*;
use worki_isolation::{Backend, LaunchRequest, Workspace};

use crate::openers::{self, OpenerSettings};
use crate::setup::{run_captured, run_logged};
use crate::store::Store;
use crate::{git, github, paths, secrets, Error, Host, Result};

const AGENT_SETTINGS_KEY: &str = "agents";
const ISOLATION_SETTINGS_KEY: &str = "isolation";
const OPENER_SETTINGS_KEY: &str = "openers";
const SSH_HOSTS_KEY: &str = "sshHosts";
const SAVED_SESSIONS_KEY: &str = "agentSessions";
/// Per-worktree isolation choices, keyed by worktree path.
const WORKTREE_ISOLATION_KEY: &str = "worktreeIsolation";
const SNAPSHOTS_KEY: &str = "snapshots";

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

    /// The environment `worktree`'s agents run in (call after `grant_access`).
    pub async fn agent_instance(
        &self,
        project: &Project,
        worktree: &Path,
        isolation: &Isolation,
    ) -> Result<Arc<dyn Backend>> {
        let backend = isolation.backend();
        let ws = Self::workspace(project, Some(worktree));
        blocking(move || Ok(Arc::from(backend.instance_for(&ws).unwrap_or(backend)))).await
    }

    /// Makes sure `executable` runs in `instance`: when it's missing there, calls `installing` and
    /// installs it with its known installer. Returns whether it installed it.
    pub async fn ensure_agent(
        &self,
        instance: Arc<dyn Backend>,
        isolation: &Isolation,
        executable: &str,
        installing: impl FnOnce(),
    ) -> Result<bool> {
        let (check, exe) = (instance.clone(), executable.to_owned());
        if blocking(move || Ok(check.which(&exe).is_some())).await? {
            return Ok(false);
        }
        let label = isolation.label();
        if !instance.capabilities().install {
            return Err(Error::Isolation(format!(
                "`{executable}` isn't installed for {label}. Open Isolation and use “Use my installation” under Agents."
            )));
        }
        installing();
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let lines = output.clone();
        self.install_in(isolation, instance.as_ref(), executable, move |line| {
            let (LogLine::Info(text) | LogLine::Stdout(text) | LogLine::Stderr(text)) = line;
            let mut lines = lines.lock().unwrap();
            lines.push(text);
            let excess = lines.len().saturating_sub(10);
            lines.drain(..excess);
        })
        .await
        .map_err(|e| {
            let tail = output.lock().unwrap().join("\n");
            Error::Isolation(format!(
                "Couldn't install `{executable}` for {label}: {e}\n{tail}"
            ))
        })?;
        Ok(true)
    }

    /// Answers `executable`'s trust question for `worktree` in `instance` (see
    /// `agents::trust_worktree`).
    pub async fn trust_worktree(
        &self,
        instance: Arc<dyn Backend>,
        executable: &str,
        worktree: &Path,
    ) -> Result<()> {
        let (exe, worktree) = (executable.to_owned(), worktree.to_owned());
        blocking(move || crate::agents::trust_worktree(instance.as_ref(), &exe, &worktree)).await
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
            stdin: false,
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

    pub fn saved_sessions(&self) -> Result<Vec<SavedSession>> {
        self.setting(SAVED_SESSIONS_KEY)
    }

    pub fn save_session(&self, session: SavedSession) -> Result<()> {
        let mut saved = self.saved_sessions()?;
        saved.retain(|s| s.id != session.id);
        saved.push(session);
        self.store.set_setting(SAVED_SESSIONS_KEY, &saved)
    }

    pub fn forget_session(&self, id: &str) -> Result<()> {
        let mut saved = self.saved_sessions()?;
        let before = saved.len();
        saved.retain(|s| s.id != id);
        if saved.len() == before {
            return Ok(());
        }
        self.store.set_setting(SAVED_SESSIONS_KEY, &saved)
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
    /// Saves the worktree's container as an image or sbx template named `tag`, and with
    /// `use_for_project` makes the project create new worktree environments from it.
    pub async fn save_environment_template(
        &self,
        project_id: &str,
        worktree: &Path,
        tag: &str,
        use_for_project: bool,
    ) -> Result<Project> {
        let project = self.store.get_project(project_id)?;
        let backend = self.isolation_for_worktree(&project, worktree)?.backend();
        let worktree = worktree.to_path_buf();
        let tag = tag.trim().to_owned();
        let saved = blocking(move || backend.save_template(&worktree, &tag)).await?;
        if let Some(snapshot) = SnapshotEngine::snapshot_of(&saved) {
            let mut recorded: Vec<(SnapshotEngine, String)> = self.setting(SNAPSHOTS_KEY)?;
            if !recorded.contains(&snapshot) {
                recorded.push(snapshot);
                self.store.set_setting(SNAPSHOTS_KEY, &recorded)?;
            }
        }
        if !use_for_project {
            return Ok(project);
        }
        let isolation = match saved {
            Isolation::DockerSandbox { image, template } => {
                ProjectIsolation::DockerSandbox { image, template }
            }
            Isolation::Docker { image } => ProjectIsolation::Docker { image },
            Isolation::Podman { image } => ProjectIsolation::Podman { image },
            other => {
                return Err(Error::Isolation(format!(
                    "{} can't be a project default",
                    other.label()
                )))
            }
        };
        self.update_project(
            project_id,
            ProjectPatch {
                isolation: Some(isolation),
                ..Default::default()
            },
        )
    }

    pub async fn environment_templates(&self, isolation: Isolation) -> Result<Vec<String>> {
        blocking(move || isolation.backend().templates()).await
    }

    /// Snapshots on this machine: ones worki saved or named `worki-…`, and ones a setting starts
    /// environments from, newest first. Engines that aren't running are left out.
    pub async fn snapshots(&self) -> Result<Vec<Snapshot>> {
        let users = self.snapshot_users()?;
        let recorded: Vec<(SnapshotEngine, String)> = self.setting(SNAPSHOTS_KEY)?;
        let found = blocking(|| {
            Ok(SnapshotEngine::ALL
                .into_iter()
                .flat_map(|engine| {
                    let names = engine
                        .isolation()
                        .backend()
                        .dated_templates()
                        .unwrap_or_default();
                    names
                        .into_iter()
                        .map(move |(name, created)| (engine, name, created))
                })
                .collect::<Vec<_>>())
        })
        .await?;
        let mut snapshots: Vec<Snapshot> = found
            .into_iter()
            .filter(|(engine, name, _)| {
                let key = (*engine, name.clone());
                (name.starts_with("worki-") && name != worki_isolation::DEFAULT_CONTAINER_IMAGE)
                    || recorded.contains(&key)
                    || users.iter().any(|(used, _)| *used == key)
            })
            .map(|(engine, name, created)| Snapshot {
                used_by: users
                    .iter()
                    .filter(|((e, n), _)| *e == engine && *n == name)
                    .map(|(_, by)| by.clone())
                    .collect(),
                engine,
                name,
                created,
            })
            .collect();
        snapshots
            .sort_by(|a, b| (b.created, &b.name, b.engine).cmp(&(a.created, &a.name, a.engine)));
        snapshots.dedup_by(|a, b| a.engine == b.engine && a.name == b.name);
        Ok(snapshots)
    }

    /// Removes a snapshot no setting starts environments from anymore.
    pub async fn delete_snapshot(&self, engine: SnapshotEngine, name: &str) -> Result<()> {
        let key = (engine, name.to_owned());
        let users: Vec<String> = self
            .snapshot_users()?
            .into_iter()
            .filter(|(used, _)| *used == key)
            .map(|(_, by)| by)
            .collect();
        if !users.is_empty() {
            return Err(Error::Isolation(format!(
                "{name} is still used by {}; pick another isolation there first",
                users.join(", ")
            )));
        }
        let tag = name.to_owned();
        blocking(move || engine.isolation().backend().delete_template(&tag)).await?;
        let mut recorded: Vec<(SnapshotEngine, String)> = self.setting(SNAPSHOTS_KEY)?;
        recorded.retain(|r| *r != key);
        self.store.set_setting(SNAPSHOTS_KEY, &recorded)
    }

    /// Every setting that starts new environments from a snapshot, with who it belongs to.
    fn snapshot_users(&self) -> Result<Vec<((SnapshotEngine, String), String)>> {
        let global = self.isolation_settings()?.default;
        let projects: Vec<Project> = self
            .list_projects()?
            .into_iter()
            .filter(|p| p.remote.is_none())
            .collect();
        let mut users = Vec::new();
        users.extend(
            SnapshotEngine::snapshot_of(&global).map(|s| (s, "The default isolation".to_owned())),
        );
        for p in &projects {
            if p.isolation != ProjectIsolation::Inherit {
                users.extend(
                    SnapshotEngine::snapshot_of(&p.isolation.resolve(&global))
                        .map(|s| (s, p.name.clone())),
                );
            }
        }
        for (path, choice) in self.worktree_choices()? {
            let path = Path::new(&path);
            let Some(project) = projects.iter().find(|p| p.contains(path)) else {
                continue;
            };
            if choice != ProjectIsolation::Inherit {
                let worktree = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                users.extend(
                    SnapshotEngine::snapshot_of(&choice.resolve(&global))
                        .map(|s| (s, format!("{} › {worktree}", project.name))),
                );
            }
        }
        Ok(users)
    }

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

    pub async fn commit(&self, worktree: &Path, files: &[String], message: &str) -> Result<String> {
        git::commit(&self.host_for_path(worktree)?, worktree, files, message).await
    }

    pub async fn branch_log(&self, worktree: &Path, base: Option<&str>) -> Result<Vec<CommitInfo>> {
        git::branch_log(&self.host_for_path(worktree)?, worktree, base).await
    }

    pub async fn compare_base(&self, worktree: &Path, base: &str) -> Result<String> {
        Ok(git::compare_base(&self.host_for_path(worktree)?, worktree, base).await)
    }

    pub async fn fetch(&self, worktree: &Path) -> Result<()> {
        git::fetch(&self.host_for_path(worktree)?, worktree).await
    }

    pub async fn update_from_base(&self, worktree: &Path, base: &str) -> Result<BaseUpdate> {
        git::update_from_base(&self.host_for_path(worktree)?, worktree, base).await
    }

    pub async fn pull_request(&self, worktree: &Path) -> Result<Option<PullRequest>> {
        github::pull_request(&self.host_for_path(worktree)?, worktree).await
    }

    pub async fn create_pull_request(
        &self,
        worktree: &Path,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<PullRequest> {
        let host = self.host_for_path(worktree)?;
        github::create_pull_request(&host, worktree, base, title, body, draft).await
    }

    pub async fn push(&self, worktree: &Path) -> Result<()> {
        github::push(&self.host_for_path(worktree)?, worktree).await
    }

    pub async fn pr_review_comments(&self, worktree: &Path, number: u64) -> Result<Vec<PrComment>> {
        github::review_comments(&self.host_for_path(worktree)?, worktree, number).await
    }

    pub async fn github_image(&self, worktree: &Path, url: &str) -> Result<String> {
        github::attachment_image(&self.host_for_path(worktree)?, worktree, url).await
    }

    pub async fn pr_failed_checks(&self, worktree: &Path, number: u64) -> Result<Vec<FailedCheck>> {
        github::failed_checks(&self.host_for_path(worktree)?, worktree, number).await
    }

    /// Has an agent write the pull request's title and description following `instructions`.
    pub async fn draft_pull_request(
        &self,
        worktree: &Path,
        base: &str,
        instructions: &str,
        writer: &Writer,
    ) -> Result<PrDraft> {
        let context = git::pr_context(&self.host_for_path(worktree)?, worktree, base).await?;
        let (name, answer) = self
            .write_once(
                worktree,
                writer,
                crate::prompt::fill(instructions, &context, github::DRAFT_ANSWER),
            )
            .await?;
        github::parse_draft(&answer).ok_or_else(|| {
            Error::Isolation(format!("{name} didn't answer with a title and description"))
        })
    }

    /// Has an agent split the uncommitted changes to `files` into commits and write their
    /// messages.
    pub async fn draft_commits(
        &self,
        worktree: &Path,
        files: &[String],
        instructions: &str,
        writer: &Writer,
    ) -> Result<Vec<CommitDraft>> {
        let context = git::commit_context(&self.host_for_path(worktree)?, worktree, files).await?;
        let (name, answer) = self
            .write_once(
                worktree,
                writer,
                crate::prompt::fill(instructions, &context, git::COMMITS_ANSWER),
            )
            .await?;
        git::parse_commits(&answer, files)
            .ok_or_else(|| Error::Isolation(format!("{name} didn't answer with commits")))
    }

    /// Runs the writer once in the worktree's isolation on `prompt`; returns the agent's name
    /// and its answer.
    async fn write_once(
        &self,
        worktree: &Path,
        writer: &Writer,
        prompt: String,
    ) -> Result<(String, String)> {
        let project = self
            .list_projects()?
            .into_iter()
            .find(|p| p.contains(worktree))
            .ok_or_else(|| Error::ProjectNotFound(worktree.display().to_string()))?;
        let template = writer
            .command
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .unwrap_or(crate::launch::DEFAULT_WRITER_COMMAND);
        let name = template
            .split_whitespace()
            .next()
            .unwrap_or("The agent")
            .to_owned();
        let command = crate::launch::writer_command(template, writer.fork.as_deref());
        let isolation = self.grant_access(&project, Some(worktree)).await?;
        let mut env = self.secret_env(&isolation)?;
        env.push(("WORKI_PROMPT".into(), prompt));
        let req = LaunchRequest {
            cwd: worktree.to_path_buf(),
            command: Some(command),
            interactive: false,
            stdin: false,
            env,
            session: None,
        };
        let launch = blocking(move || isolation.backend().launch(&req)).await?;
        Ok((name, run_captured(launch).await?))
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
        git::set_branch_base(host, repo, branch, base).await?;

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
        stdin: false,
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
