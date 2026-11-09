use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::model::Project;
use crate::{Error, Result};

const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE projects (
        id            TEXT PRIMARY KEY,
        name          TEXT NOT NULL,
        repo_path     TEXT NOT NULL UNIQUE,
        worktree_root TEXT NOT NULL,
        default_base  TEXT NOT NULL,
        run_as        TEXT NOT NULL,
        setup_script  TEXT,
        position      INTEGER NOT NULL,
        created_at    INTEGER NOT NULL DEFAULT (unixepoch())
    );
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
"#,
    r#"
    CREATE TABLE projects_v2 (
        id            TEXT PRIMARY KEY,
        name          TEXT NOT NULL,
        repo_path     TEXT NOT NULL,
        worktree_root TEXT NOT NULL,
        default_base  TEXT NOT NULL,
        run_as        TEXT NOT NULL,
        setup_script  TEXT,
        position      INTEGER NOT NULL,
        created_at    INTEGER NOT NULL DEFAULT (unixepoch()),
        remote        TEXT
    );
    INSERT INTO projects_v2 (id, name, repo_path, worktree_root, default_base, run_as, setup_script, position, created_at)
        SELECT id, name, repo_path, worktree_root, default_base, run_as, setup_script, position, created_at FROM projects;
    DROP TABLE projects;
    ALTER TABLE projects_v2 RENAME TO projects;
"#,
];

const SELECT_PROJECT: &str =
    "SELECT id, name, repo_path, worktree_root, default_base, run_as, setup_script, remote FROM projects";

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
            conn.execute_batch(sql)?;
            conn.pragma_update(None, "user_version", i + 1)?;
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn list_projects(&self) -> Result<Vec<Project>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{SELECT_PROJECT} ORDER BY position, created_at"))?;
        let rows = stmt.query_map([], row_to_project)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get_project(&self, id: &str) -> Result<Project> {
        self.conn()
            .query_row(
                &format!("{SELECT_PROJECT} WHERE id = ?1"),
                [id],
                row_to_project,
            )
            .optional()?
            .ok_or_else(|| Error::ProjectNotFound(id.into()))
    }

    /// The project for `repo` on `host` (`user@host:port` as displayed; `None` for this Mac).
    pub fn find_by_repo(&self, repo: &Path, host: Option<&str>) -> Result<Option<String>> {
        Ok(self
            .list_projects()?
            .into_iter()
            .find(|p| {
                p.repo_path == repo
                    && p.remote.as_ref().map(|r| r.target().display()).as_deref() == host
            })
            .map(|p| p.id))
    }

    pub fn insert_project(&self, p: &Project) -> Result<()> {
        self.conn().execute(
            "INSERT INTO projects (id, name, repo_path, worktree_root, default_base, run_as, setup_script, remote, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, (SELECT COALESCE(MAX(position), -1) + 1 FROM projects))",
            params![
                p.id,
                p.name,
                p.repo_path.to_string_lossy(),
                p.worktree_root.to_string_lossy(),
                p.default_base,
                serde_json::to_string(&p.isolation)?,
                p.setup_script,
                p.remote.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        Ok(())
    }

    pub fn update_project(&self, p: &Project) -> Result<()> {
        let changed = self.conn().execute(
            "UPDATE projects SET name = ?2, worktree_root = ?3, default_base = ?4, run_as = ?5, setup_script = ?6,
             remote = ?7 WHERE id = ?1",
            params![
                p.id,
                p.name,
                p.worktree_root.to_string_lossy(),
                p.default_base,
                serde_json::to_string(&p.isolation)?,
                p.setup_script,
                p.remote.as_ref().map(serde_json::to_string).transpose()?,
            ],
        )?;
        if changed == 0 {
            return Err(Error::ProjectNotFound(p.id.clone()));
        }
        Ok(())
    }

    pub fn delete_project(&self, id: &str) -> Result<()> {
        self.conn()
            .execute("DELETE FROM projects WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn get_setting<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let value: Option<String> = self
            .conn()
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(value.map(|v| serde_json::from_str(&v)).transpose()?)
    }

    pub fn set_setting<T: serde::Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    pub fn reorder_projects(&self, ids: &[String]) -> Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (pos, id) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE projects SET position = ?2 WHERE id = ?1",
                params![id, pos],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

fn json_column<T: serde::de::DeserializeOwned>(r: &Row, i: usize) -> rusqlite::Result<Option<T>> {
    let Some(text) = r.get::<_, Option<String>>(i)? else {
        return Ok(None);
    };
    serde_json::from_str(&text).map(Some).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, Box::new(e))
    })
}

fn row_to_project(r: &Row) -> rusqlite::Result<Project> {
    let isolation = json_column(r, 5)?.unwrap_or_default();
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        repo_path: PathBuf::from(r.get::<_, String>(2)?),
        worktree_root: PathBuf::from(r.get::<_, String>(3)?),
        default_base: r.get(4)?,
        isolation,
        setup_script: r.get(6)?,
        remote: json_column(r, 7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use worki_isolation::ProjectIsolation;

    fn project(id: &str, repo: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            repo_path: repo.into(),
            worktree_root: "/w".into(),
            default_base: "main".into(),
            isolation: ProjectIsolation::MacUser {
                user: "_worki_x".into(),
            },
            setup_script: Some("echo hi".into()),
            remote: None,
        }
    }

    #[test]
    fn crud_roundtrip() {
        let store = Store::in_memory().unwrap();
        store.insert_project(&project("a", "/r/a")).unwrap();
        store.insert_project(&project("b", "/r/b")).unwrap();
        let mut remote = project("c", "/r/a");
        remote.remote = Some(crate::model::Remote {
            host: "devbox".into(),
            user: None,
            port: None,
            keep_alive: true,
        });
        store.insert_project(&remote).unwrap();
        assert_eq!(
            store
                .find_by_repo(Path::new("/r/a"), Some("devbox"))
                .unwrap()
                .as_deref(),
            Some("c")
        );
        assert_eq!(
            store
                .find_by_repo(Path::new("/r/a"), None)
                .unwrap()
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            store.get_project("c").unwrap().remote.unwrap().host,
            "devbox"
        );
        store.delete_project("c").unwrap();

        let mut a = store.get_project("a").unwrap();
        assert_eq!(
            a.isolation,
            ProjectIsolation::MacUser {
                user: "_worki_x".into()
            }
        );
        a.name = "renamed".into();
        a.setup_script = None;
        store.update_project(&a).unwrap();
        let a = store.get_project("a").unwrap();
        assert_eq!((a.name.as_str(), a.setup_script), ("renamed", None));

        store.reorder_projects(&["b".into(), "a".into()]).unwrap();
        let ids: Vec<_> = store
            .list_projects()
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(ids, ["b", "a"]);

        assert_eq!(
            store
                .find_by_repo(Path::new("/r/b"), None)
                .unwrap()
                .as_deref(),
            Some("b")
        );

        assert_eq!(store.get_setting::<Vec<u8>>("k").unwrap(), None);
        store.set_setting("k", &vec![1u8, 2]).unwrap();
        store.set_setting("k", &vec![3u8]).unwrap();
        assert_eq!(store.get_setting::<Vec<u8>>("k").unwrap(), Some(vec![3]));
        store.delete_project("b").unwrap();
        assert!(matches!(
            store.get_project("b"),
            Err(Error::ProjectNotFound(_))
        ));
    }
}
