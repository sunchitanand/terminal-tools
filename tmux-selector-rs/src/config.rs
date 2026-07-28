//! Local project config, stored at ~/.tmux-projects.json.
//! Format is a JSON object keyed by host, each value an ordered map of
//! session-name -> working-directory. Fully compatible with the zsh script.

use anyhow::Result;
use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;

pub struct Config {
    path: PathBuf,
    host: String,
    root: Map<String, Value>,
}

impl Config {
    pub fn load(host: &str) -> Self {
        Self::load_from(config_path(), host)
    }

    /// Load from an explicit path (used by tests, and by `load`).
    pub fn load_from(path: PathBuf, host: &str) -> Self {
        let root = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        Config {
            path,
            host: host.to_string(),
            root,
        }
    }

    /// Ordered (name, dir) pairs for this host, preserving insertion order.
    pub fn entries(&self) -> Vec<(String, String)> {
        self.root
            .get(&self.host)
            .and_then(|v| v.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn host_map_mut(&mut self) -> &mut Map<String, Value> {
        self.root
            .entry(self.host.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        self.root
            .get_mut(&self.host)
            .and_then(|v| v.as_object_mut())
            .expect("host entry is an object")
    }

    /// Insert or update a session. A non-empty dir always overwrites; an empty
    /// dir only creates the key if it does not already exist (matches script).
    pub fn upsert(&mut self, name: &str, dir: &str) {
        let host = self.host_map_mut();
        if !dir.is_empty() {
            host.insert(name.to_string(), Value::String(dir.to_string()));
        } else if !host.contains_key(name) {
            host.insert(name.to_string(), Value::String(String::new()));
        }
    }

    pub fn remove(&mut self, name: &str) {
        if let Some(host) = self
            .root
            .get_mut(&self.host)
            .and_then(|v| v.as_object_mut())
        {
            host.remove(name);
        }
    }

    pub fn save(&self) -> Result<()> {
        let s = serde_json::to_string_pretty(&Value::Object(self.root.clone()))?;
        fs::write(&self.path, s)?;
        Ok(())
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".tmux-projects.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp(name: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("tmux-sel-test-{name}.json"));
        p
    }

    #[test]
    fn preserves_other_hosts_and_order_on_save() {
        let path = tmp("multihost");
        let seed = r#"{
  "hostA": {"proj/a": "/a", "proj/b": ""},
  "hostB": {"x/y": "/xy"}
}"#;
        fs::write(&path, seed).unwrap();

        let mut cfg = Config::load_from(path.clone(), "hostA");
        // Simulate the write-back loop: re-upsert existing entries unchanged.
        for (n, d) in cfg.entries() {
            cfg.upsert(&n, &d);
        }
        cfg.save().unwrap();

        // hostB must survive untouched.
        let reload: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reload["hostB"]["x/y"], "/xy");
        // hostA order preserved (preserve_order feature).
        let keys: Vec<&str> = reload["hostA"]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, vec!["proj/a", "proj/b"]);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn empty_dir_does_not_clobber_existing() {
        let path = tmp("noclobber");
        fs::write(&path, r#"{"h": {"s": "/real/dir"}}"#).unwrap();
        let mut cfg = Config::load_from(path.clone(), "h");
        cfg.upsert("s", ""); // empty dir should NOT overwrite
        cfg.save().unwrap();
        let reload: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(reload["h"]["s"], "/real/dir");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn remove_deletes_key() {
        let path = tmp("remove");
        fs::write(&path, r#"{"h": {"a": "/a", "b": "/b"}}"#).unwrap();
        let mut cfg = Config::load_from(path.clone(), "h");
        cfg.remove("a");
        cfg.save().unwrap();
        let reload: Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(reload["h"].get("a").is_none());
        assert_eq!(reload["h"]["b"], "/b");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn missing_file_yields_empty() {
        let cfg = Config::load_from(tmp("does-not-exist-xyz"), "h");
        assert!(cfg.entries().is_empty());
    }
}
