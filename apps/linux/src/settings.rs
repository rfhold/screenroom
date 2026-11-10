use screenroom_client_core::{ClientSettings, default_settings, validate_settings};
use std::{fs, io::Write, path::PathBuf};

pub fn path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SCREENROOM_SETTINGS_PATH") {
        return Ok(path.into());
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or("No configuration directory")?;
    let namespace = std::env::var("SCREENROOM_E2E_SETTINGS_NAMESPACE")
        .or_else(|_| std::env::var("E2E_PROFILE_PREFIX"))
        .unwrap_or_default();
    if !namespace
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
    {
        return Err("Invalid local settings namespace".into());
    }
    Ok(base.join("screenroom").join(if namespace.is_empty() {
        "settings.json".into()
    } else {
        format!("settings-{namespace}.json")
    }))
}

pub fn load() -> ClientSettings {
    let result = path().and_then(load_at);
    result.unwrap_or_else(|e: String| {
        eprintln!("Local settings unavailable; using defaults: {e}");
        default_settings()
    })
}
fn load_at(path: PathBuf) -> Result<ClientSettings, String> {
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default_settings()),
        Err(e) => return Err(e.to_string()),
    };
    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if value["version"] != 1 {
        return Err("Unsupported settings version".into());
    }
    let settings = serde_json::from_value(value["settings"].clone()).map_err(|e| e.to_string())?;
    validate_settings(settings).map_err(|e| e.to_string())
}

pub fn save(settings: ClientSettings) -> Result<(), String> {
    save_at(path()?, settings)
}
fn save_at(path: PathBuf, settings: ClientSettings) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let settings = validate_settings(settings).map_err(|e| e.to_string())?;
    let parent = path.parent().ok_or("Missing settings directory")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)
        .map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(
            &serde_json::json!({"version":1,"settings":settings}),
        )?)?;
        file.sync_all()?;
        fs::rename(&temp, &path)?;
        fs::File::open(parent)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result.map_err(|e: std::io::Error| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn versioned_atomic_settings_are_private_validated_and_isolated() {
        use std::os::unix::fs::PermissionsExt;
        let directory =
            std::env::temp_dir().join(format!("screenroom-settings-test-{}", std::process::id()));
        let path = directory.join("settings.json");
        let other = directory.join("isolated.json");
        let mut settings = default_settings();
        settings.subtitle_preference = "transcription".into();
        settings.source = "tmdb".into();
        settings.scope.as_mut().unwrap().source_id = "mock-tmdb".into();
        settings.scope.as_mut().unwrap().library_id = "movies".into();
        settings.types.as_mut().unwrap().movie = false;
        settings.types.as_mut().unwrap().episode = false;
        settings.methods.as_mut().unwrap().title_only = true;
        settings.methods.as_mut().unwrap().text = false;
        settings.methods.as_mut().unwrap().semantic = false;
        save_at(path.clone(), settings.clone()).unwrap();
        assert_eq!(load_at(path.clone()).unwrap(), settings);
        assert_eq!(load_at(other).unwrap(), default_settings());
        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::write(&path, r#"{"version":2,"settings":{}}"#).unwrap();
        assert!(load_at(path.clone()).is_err());
        settings.source = "invalid".into();
        assert!(save_at(path.clone(), settings).is_err());
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
