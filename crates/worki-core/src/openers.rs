//! Apps a worktree can be opened in ("Open in VS Code", "Open in IntelliJ IDEA", …).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenerApp {
    pub id: String,
    pub name: String,
    /// Path of the `.app` bundle.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OpenerSettings {
    pub apps: Vec<OpenerApp>,
    pub default_app: Option<String>,
}

/// Well-known apps, by display name and bundle names to look for (first found wins).
const KNOWN: &[(&str, &[&str])] = &[
    ("VS Code", &["Visual Studio Code.app"]),
    ("Cursor", &["Cursor.app"]),
    ("Zed", &["Zed.app"]),
    (
        "IntelliJ IDEA",
        &[
            "IntelliJ IDEA.app",
            "IntelliJ IDEA Ultimate.app",
            "IntelliJ IDEA CE.app",
        ],
    ),
    ("WebStorm", &["WebStorm.app"]),
    (
        "PyCharm",
        &[
            "PyCharm.app",
            "PyCharm Professional Edition.app",
            "PyCharm CE.app",
        ],
    ),
    ("RustRover", &["RustRover.app"]),
    ("GoLand", &["GoLand.app"]),
    ("Android Studio", &["Android Studio.app"]),
    ("Xcode", &["Xcode.app"]),
    ("Terminal", &["Terminal.app"]),
    ("iTerm", &["iTerm.app"]),
    ("Ghostty", &["Ghostty.app"]),
    ("Warp", &["Warp.app"]),
    ("Finder", &["Finder.app"]),
];

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
        // JetBrains Toolbox installs here.
        dirs.push(home.join("Applications/JetBrains Toolbox"));
    }
    dirs.extend(
        [
            "/System/Applications",
            "/System/Applications/Utilities",
            "/System/Library/CoreServices",
        ]
        .into_iter()
        .map(PathBuf::from),
    );
    dirs
}

fn slug(name: &str) -> String {
    name.to_ascii_lowercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect()
}

/// Installed well-known apps, in the order of [`KNOWN`].
pub fn detect_installed() -> Vec<OpenerApp> {
    detect_in(&search_dirs())
}

fn detect_in(dirs: &[PathBuf]) -> Vec<OpenerApp> {
    KNOWN
        .iter()
        .filter_map(|(name, bundles)| {
            let path = bundles
                .iter()
                .flat_map(|b| dirs.iter().map(move |d| d.join(b)))
                .find(|p| p.is_dir())?;
            Some(OpenerApp {
                id: slug(name),
                name: (*name).into(),
                path,
            })
        })
        .collect()
}

/// Display name for a user-picked bundle: `/Applications/Sublime Text.app` → `Sublime Text`.
pub fn app_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "App".into())
}

pub fn app_from_path(path: &Path) -> OpenerApp {
    let name = app_name(path);
    OpenerApp {
        id: format!("{}-{}", slug(&name), uuid::Uuid::new_v4().simple()),
        name,
        path: path.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_apps_in_given_folders() {
        let dir = tempfile::tempdir().unwrap();
        for b in [
            "Visual Studio Code.app",
            "IntelliJ IDEA CE.app",
            "Finder.app",
        ] {
            std::fs::create_dir(dir.path().join(b)).unwrap();
        }
        let apps = detect_in(&[dir.path().to_path_buf()]);
        let names: Vec<_> = apps.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["VS Code", "IntelliJ IDEA", "Finder"]);
        assert!(apps[1].path.ends_with("IntelliJ IDEA CE.app"));
        assert_eq!(apps[0].id, "vscode");
    }

    #[test]
    fn names_picked_bundles() {
        let app = app_from_path(Path::new("/Applications/Sublime Text.app"));
        assert_eq!(app.name, "Sublime Text");
        assert!(app.id.starts_with("sublimetext-"));
    }
}
