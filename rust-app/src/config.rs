use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{APP_DIR_NAME, platform};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Config {
    pub engine: String,
    pub ocrspace_key: String,
    pub ocrspace_engine: String,
    pub gemini_key: String,
    pub gemini_model: String,
    pub translation_model: String,
    pub font_family: String,
    pub main_font_size: u32,
    pub sticky_font_size: u32,
    pub camera: u32,
    pub launcher_pos: Option<[i32; 2]>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            engine: "ocrspace".into(),
            ocrspace_key: "helloworld".into(),
            ocrspace_engine: "3".into(),
            gemini_key: String::new(),
            gemini_model: "gemini-flash-latest".into(),
            translation_model: "gemini".into(),
            font_family: "Segoe UI".into(),
            main_font_size: 14,
            sticky_font_size: 18,
            camera: 0,
            launcher_pos: None,
        }
    }
}

pub fn app_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR_NAME)
}

pub fn config_path() -> PathBuf {
    app_dir().join("config.json")
}

pub fn history_dir() -> PathBuf {
    app_dir().join("history")
}

pub fn load(path: &Path) -> Config {
    let mut config: Config = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    config.ocrspace_engine = "3".into();
    config.gemini_model = "gemini-flash-latest".into();
    config
}

pub fn save(config: &Config, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("tmp");
    let bytes = serde_json::to_vec_pretty(config).map_err(io::Error::other)?;
    fs::write(&temp, bytes)?;
    platform::atomic_replace(&temp, path)
}

pub fn validate_key(name: &str, key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err(format!("{name} API key is empty"));
    }
    if key.len() > 200 || !key.is_ascii() || key.chars().any(char::is_whitespace) {
        return Err(format!(
            "{name} API key looks wrong: paste ONLY the key (one line, English letters/digits, no spaces)"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_config_uses_defaults_and_round_trips() {
        let root = std::env::temp_dir().join(format!("roi-ocr-config-{}", std::process::id()));
        let path = root.join("nested/config.json");
        let _ = fs::remove_dir_all(&root);
        assert_eq!(load(&path), Config::default());
        let config = Config {
            gemini_key: "K81234567890123".into(),
            ..Default::default()
        };
        save(&config, &path).unwrap();
        assert_eq!(load(&path), config);
        fs::write(&path, b"{broken").unwrap();
        assert_eq!(load(&path), Config::default());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn keys_are_checked_at_the_trust_boundary() {
        assert!(validate_key("Gemini", "K81234567890123").is_ok());
        assert!(validate_key("Gemini", "").is_err());
        assert!(validate_key("Gemini", "line1\nline2").is_err());
        assert!(validate_key("Gemini", "ⓘ-key").is_err());
    }
}
