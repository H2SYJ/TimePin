use serde::Serialize;

/// Describes build-time configuration, not connectivity or update availability.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdaterConfig {
    schema_version: &'static str,
    configured: bool,
}

fn config_status(plugins: &tauri::utils::config::PluginConfig) -> UpdaterConfig {
    let configured = plugins
        .0
        .get("updater")
        .and_then(|config| config.get("pubkey"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|key| !key.trim().is_empty());
    UpdaterConfig {
        schema_version: "1.0.0",
        configured,
    }
}

/// Read the embedded (including CLI build overrides) public key configuration.
/// Never return private keys or treat an unconfigured build as up to date.
#[tauri::command]
pub fn get_updater_config(app: tauri::AppHandle) -> UpdaterConfig {
    config_status(&app.config().plugins)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unconfigured_builds_are_explicit() {
        for value in [json!({}), json!({"updater":{"pubkey":" \n "}})] {
            let plugins = serde_json::from_value(value).unwrap();
            assert_eq!(
                serde_json::to_value(config_status(&plugins)).unwrap(),
                json!({"schemaVersion":"1.0.0","configured":false})
            );
        }
    }

    #[test]
    fn reads_the_merged_build_configuration() {
        let plugins =
            serde_json::from_value(json!({"updater":{"pubkey":"build override"}})).unwrap();
        assert!(config_status(&plugins).configured);
    }
}
