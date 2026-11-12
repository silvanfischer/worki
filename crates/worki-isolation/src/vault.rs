//! worki's secrets (agent tokens, agent keychain passwords) in a single item of the human's
//! login Keychain.
//!
//! macOS asks for permission when an app reads a Keychain item's *data*. So the data is read at
//! most once per process and cached, while the list of key names lives in the item's comment
//! attribute, which can be read without a prompt. Status checks therefore never prompt.
//!
//! Debug builds keep their secrets in a file only the human can read instead: without an Apple
//! Team ID, macOS ties Keychain access to the binary's hash, so it would ask after every rebuild.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::Result;

static CACHE: Mutex<Option<BTreeMap<String, String>>> = Mutex::new(None);

fn cache() -> std::sync::MutexGuard<'static, Option<BTreeMap<String, String>>> {
    CACHE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Whether `key` has a value, without reading any secret data.
pub fn has(key: &str) -> bool {
    if let Some(map) = cache().as_ref() {
        return map.contains_key(key);
    }
    imp::stored_keys().iter().any(|k| k == key)
}

pub fn get(key: &str) -> Option<String> {
    let mut cache = cache();
    if cache.is_none() {
        if !imp::exists() {
            return None;
        }
        *cache = Some(imp::read()?);
    }
    cache.as_ref().and_then(|m| m.get(key).cloned())
}

pub fn set(key: &str, value: &str) -> Result<()> {
    update(|map| {
        map.insert(key.to_owned(), value.to_owned());
    })
}

pub fn delete(key: &str) -> Result<()> {
    if !has(key) {
        return Ok(());
    }
    update(|map| {
        map.remove(key);
    })
}

fn update(change: impl FnOnce(&mut BTreeMap<String, String>)) -> Result<()> {
    let mut cache = cache();
    let mut map = match cache.take() {
        Some(map) => map,
        None if imp::exists() => {
            imp::read().ok_or_else(|| anyhow::anyhow!("could not read worki's Keychain item"))?
        }
        None => BTreeMap::new(),
    };
    change(&mut map);
    imp::write(&map)?;
    *cache = Some(map);
    Ok(())
}

#[cfg(all(target_os = "macos", not(debug_assertions)))]
mod imp {
    use std::collections::BTreeMap;

    use anyhow::Result;
    use core_foundation::data::CFData;
    use security_framework::item::{
        ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, ItemUpdateOptions,
        ItemUpdateValue, Limit, SearchResult,
    };
    use security_framework::passwords::get_generic_password;

    const SERVICE: &str = "ch.worki";
    const ACCOUNT: &str = "vault";

    fn search() -> ItemSearchOptions {
        let mut opts = ItemSearchOptions::new();
        opts.class(ItemClass::generic_password())
            .service(SERVICE)
            .account(ACCOUNT)
            .limit(Limit::Max(1));
        opts
    }

    fn attributes() -> Option<std::collections::HashMap<String, String>> {
        let results = search().load_attributes(true).search().ok()?;
        results.into_iter().find_map(|r| match r {
            SearchResult::Dict(_) => r.simplify_dict(),
            _ => None,
        })
    }

    pub fn exists() -> bool {
        attributes().is_some()
    }

    /// Key names from the comment attribute (`icmt`).
    pub fn stored_keys() -> Vec<String> {
        attributes()
            .and_then(|a| a.get("icmt").cloned())
            .map(|c| {
                c.lines()
                    .map(str::to_owned)
                    .filter(|l| !l.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn read() -> Option<BTreeMap<String, String>> {
        let data = get_generic_password(SERVICE, ACCOUNT).ok()?;
        serde_json::from_slice(&data).ok()
    }

    pub fn write(map: &BTreeMap<String, String>) -> Result<()> {
        let data = serde_json::to_vec(map)?;
        let comment = map.keys().cloned().collect::<Vec<_>>().join("\n");
        if exists() {
            let mut update = ItemUpdateOptions::new();
            update
                .set_value(ItemUpdateValue::Data(CFData::from_buffer(&data)))
                .set_comment(&comment);
            security_framework::item::update_item(&search(), &update)?;
        } else {
            let mut add = ItemAddOptions::new(ItemAddValue::Data {
                class: ItemClass::generic_password(),
                data: CFData::from_buffer(&data),
            });
            add.set_service(SERVICE)
                .set_account_name(ACCOUNT)
                .set_label("worki")
                .set_comment(&comment);
            add.add()?;
        }
        Ok(())
    }
}

#[cfg(all(debug_assertions, unix))]
mod imp {
    use std::collections::BTreeMap;
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use anyhow::Result;

    fn path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("worki")
            .join("dev-secrets.json")
    }

    pub fn exists() -> bool {
        path().exists()
    }

    pub fn stored_keys() -> Vec<String> {
        read().map(|m| m.into_keys().collect()).unwrap_or_default()
    }

    pub fn read() -> Option<BTreeMap<String, String>> {
        read_from(&path())
    }

    pub fn write(map: &BTreeMap<String, String>) -> Result<()> {
        write_to(&path(), map)
    }

    fn read_from(path: &Path) -> Option<BTreeMap<String, String>> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    fn write_to(path: &Path, map: &BTreeMap<String, String>) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(map)?)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn keeps_secrets_in_a_file_only_the_owner_can_read() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("worki").join("dev-secrets.json");
            let map = BTreeMap::from([("TOKEN".to_owned(), "s3cret".to_owned())]);
            write_to(&path, &map).unwrap();
            assert_eq!(read_from(&path), Some(map));
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            assert!(!path.with_extension("json.tmp").exists());
        }
    }
}

#[cfg(not(any(
    all(debug_assertions, unix),
    all(target_os = "macos", not(debug_assertions))
)))]
mod imp {
    use std::collections::BTreeMap;

    pub fn exists() -> bool {
        false
    }
    pub fn stored_keys() -> Vec<String> {
        Vec::new()
    }
    pub fn read() -> Option<BTreeMap<String, String>> {
        None
    }
    pub fn write(_: &BTreeMap<String, String>) -> anyhow::Result<()> {
        anyhow::bail!("the vault is macOS-only")
    }
}
