//! Runtime restrictions for the separately identified meeting test bundle.
use tauri::{AppHandle, Manager};

pub const IDENTIFIER: &str = "com.pais.handy.meetings-dev";

/// Keep the meeting profile separate when reusing the authorized screenshot
/// test shell. NSBundle retains its signed identity for macOS permissions;
/// Tauri uses this profile for data paths, logs and single-instance IPC.
pub fn isolate_config(config: &mut tauri::Config) {
    if config.product_name.as_deref() == Some("Handy Meetings Dev")
        && config.identifier == "com.pais.handy.screenshot-test"
    {
        config.identifier = IDENTIFIER.into();
    }
}

pub fn is_meetings_dev(app: &AppHandle) -> bool {
    app.config().identifier == IDENTIFIER
}

pub fn default_settings(app: &AppHandle) -> crate::settings::AppSettings {
    let mut settings = crate::settings::get_default_settings();
    restrict_settings(app, &mut settings);
    settings
}

/// The test bundle may read the selected provider's existing credential in
/// place. It never opens the source with the store plugin or writes it back.
pub fn existing_api_key(app: &AppHandle, provider_id: &str) -> Option<String> {
    if !is_meetings_dev(app) {
        return None;
    }
    let source = app
        .path()
        .home_dir()
        .ok()?
        .join("Library/Application Support/com.pais.handy.screenshot-test/settings_store.json");
    let bytes = std::fs::read(source).ok()?;
    let store: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let settings = store.get("settings")?;
    if settings.get("post_process_provider_id")?.as_str()? != provider_id {
        return None;
    }
    settings
        .get("post_process_api_keys")?
        .get(provider_id)?
        .as_str()
        .map(str::to_owned)
}

/// Test shortcuts are deliberately far from Handy's normal defaults.
pub fn restrict_settings(app: &AppHandle, settings: &mut crate::settings::AppSettings) {
    if !is_meetings_dev(app) {
        return;
    }
    settings.autostart_enabled = false;
    settings.update_checks_enabled = false;
    settings.always_on_microphone = false;
    // Existing stores keep configurable test bindings. Only replace the
    // production defaults that are otherwise added by settings migrations.
    for (id, key) in [
        ("transcribe", "ctrl+alt+shift+super+d"),
        ("transcribe_with_post_process", "ctrl+alt+shift+super+p"),
        ("meeting", "ctrl+alt+shift+super+m"),
    ] {
        if let Some(binding) = settings.bindings.get_mut(id) {
            if binding.current_binding == binding.default_binding {
                binding.current_binding = key.into();
                binding.default_binding = key.into();
            }
        }
    }
}

pub fn log_paths(app: &AppHandle) {
    if is_meetings_dev(app) {
        log::info!(
            "Meetings test isolation: identifier={}, data={:?}, logs={:?}, config={:?}, cache={:?}",
            app.config().identifier,
            crate::portable::app_data_dir(app),
            crate::portable::app_log_dir(app),
            app.path().app_config_dir(),
            app.path().app_cache_dir()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_shell_isolates_only_the_meeting_profile() {
        let mut config = tauri::Config {
            identifier: "com.pais.handy.screenshot-test".into(),
            product_name: Some("Handy Screenshot Test".into()),
            ..Default::default()
        };
        isolate_config(&mut config);
        assert_eq!(config.identifier, "com.pais.handy.screenshot-test");
        config.product_name = Some("Handy Meetings Dev".into());
        isolate_config(&mut config);
        assert_eq!(config.identifier, IDENTIFIER);
    }
}
