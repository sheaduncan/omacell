//! Sparse, comment-preserving edits to the user `config.toml`.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use omacell_core::error::CoreError;
use toml_edit::{DocumentMut, Item, Table, value};

use crate::error;

/// Upsert `[ai] enabled` and `[ai.providers.<name>]` blocks. Preserves comments
/// and unrelated keys. Never writes secrets.
pub fn patch_ai_setup(
    path: &Path,
    enabled: bool,
    providers: &[(&str, &str, &str)],
) -> Result<(), CoreError> {
    let mut doc = if path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|err| error::io(err.to_string()))?;
        let _config_grammar: toml::Value =
            toml::from_str(&text).map_err(|err| error::schema(err.to_string()))?;
        text.parse::<DocumentMut>()
            .map_err(|err| error::schema(err.to_string()))?
    } else {
        DocumentMut::new()
    };

    if providers.is_empty() && !enabled {
        return Ok(());
    }

    if doc.get("ai").is_none() {
        doc["ai"] = Item::Table(Table::new());
    }
    let ai = doc["ai"]
        .as_table_mut()
        .ok_or_else(|| error::schema("[ai] is not a table"))?;
    if enabled {
        ai["enabled"] = value(true);
    }

    if !providers.is_empty() {
        if ai.get("providers").is_none() {
            ai["providers"] = Item::Table(Table::new());
        }
        let providers_table = ai["providers"]
            .as_table_mut()
            .ok_or_else(|| error::schema("[ai.providers] is not a table"))?;
        for (name, kind, endpoint) in providers {
            if providers_table.get(name).is_none() {
                providers_table[*name] = Item::Table(Table::new());
            }
            let block = providers_table[*name]
                .as_table_mut()
                .ok_or_else(|| error::schema(format!("[ai.providers.{name}] is not a table")))?;
            block["kind"] = value(*kind);
            block["endpoint"] = value(*endpoint);
            block["local"] = value(true);
            // Setup never creates secret references, but an endpoint refresh must
            // not discard references the user already configured.
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| error::io(err.to_string()))?;
    }
    atomic_write(path, doc.to_string().as_bytes())
}

fn atomic_write(path: &Path, body: &[u8]) -> Result<(), CoreError> {
    use std::io::Write;
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| error::io("config path has no parent"))?;
    let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
    let tmp = parent.join(format!(".omacell-ai-setup-{}-{nonce}", std::process::id()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(body)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        std::fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|err| error::io(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn patch_preserves_comments_and_never_writes_secrets() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "# keep me\nschema = 1\n[ai]\nenabled = false\n[ai.providers.ollama]\nkind = \"openai_compatible\"\nendpoint = \"http://old\"\nsecret_env = \"DROP\"\n",
        )
        .unwrap();
        patch_ai_setup(
            &path,
            true,
            &[("ollama", "openai_compatible", "http://127.0.0.1:11434/v1")],
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# keep me"), "{text}");
        assert!(text.contains("schema = 1"), "{text}");
        assert!(text.contains("enabled = true"), "{text}");
        assert!(text.contains("http://127.0.0.1:11434/v1"), "{text}");
        assert!(text.contains("secret_env = \"DROP\""), "{text}");
        assert!(!text.contains("secret_cmd"), "{text}");
    }

    #[test]
    fn empty_patch_does_not_disable_or_create() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.toml");
        patch_ai_setup(&path, false, &[]).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn patch_rejects_toml_newer_than_the_config_loader() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let original = "schema = 1\nmetadata = {\n  value = 1,\n}\n";
        assert!(toml::from_str::<toml::Value>(original).is_err());
        std::fs::write(&path, original).unwrap();

        let error = patch_ai_setup(
            &path,
            true,
            &[("ollama", "openai_compatible", "http://127.0.0.1:11434/v1")],
        )
        .unwrap_err();

        assert_eq!(error.code, "config.schema");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    }
}
