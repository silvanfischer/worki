//! worki's secrets (agent tokens, agent keychain passwords) in a single item of the human's
//! login Keychain.
//!
//! macOS asks for permission when an app reads a Keychain item's *data*, and unsigned dev builds
//! count as a new app after every rebuild. So the data is read at most once per process and
//! cached, while the list of key names lives in the item's comment attribute, which can be read
//! without a prompt. Status checks therefore never prompt.

use std::collections::BTreeMap;
use std::sync::Mutex;

use anyhow::Result;

const SERVICE: &str = "ch.worki";
const ACCOUNT: &str = "vault";

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

#[cfg(target_os = "macos")]
mod imp {
    use std::collections::BTreeMap;

    use anyhow::Result;
    use core_foundation::data::CFData;
    use security_framework::item::{
        ItemAddOptions, ItemAddValue, ItemClass, ItemSearchOptions, ItemUpdateOptions,
        ItemUpdateValue, Limit, SearchResult,
    };
    use security_framework::passwords::get_generic_password;

    use super::{ACCOUNT, SERVICE};

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

#[cfg(not(target_os = "macos"))]
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
