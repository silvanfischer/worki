//! Agent secrets (API tokens) live in worki's Keychain vault, one set per isolation scope, and
//! are only read when a session starts. Status checks never read values, so they never prompt.

use worki_isolation::vault;

use crate::{Error, Result};

fn key(scope: &str, name: &str) -> String {
    format!("env:{scope}:{name}")
}

/// Values stored before secrets were scoped.
fn legacy_key(name: &str) -> String {
    format!("env:{name}")
}

pub fn is_set(scope: &str, name: &str) -> bool {
    vault::has(&key(scope, name))
}

pub fn set(scope: &str, name: &str, value: &str) -> Result<()> {
    vault::set(&key(scope, name), value).map_err(Error::isolation)
}

pub fn delete(scope: &str, name: &str) -> Result<()> {
    vault::delete(&key(scope, name)).map_err(Error::isolation)
}

/// Moves an unscoped value into `scope`, if there is one.
pub fn adopt_legacy(scope: &str, name: &str) -> Result<()> {
    if vault::has(&legacy_key(name)) {
        if let Some(value) = vault::get(&legacy_key(name)) {
            set(scope, name, &value)?;
        }
        vault::delete(&legacy_key(name)).map_err(Error::isolation)?;
    }
    Ok(())
}

/// `(name, value)` pairs for the names that have a stored value in `scope`.
pub fn env(scope: &str, names: &[String]) -> Vec<(String, String)> {
    names
        .iter()
        .filter(|n| is_set(scope, n))
        .filter_map(|n| vault::get(&key(scope, n)).map(|v| (n.clone(), v)))
        .collect()
}
