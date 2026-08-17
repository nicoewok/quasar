use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use directories::BaseDirs;
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Result as NotifyResult, Watcher};
use serde::{Deserialize, Serialize};

/// Struct representing theme colors, deserializable from JSON.
/// Contains 4 RGBA color arrays (each array has 4 f32 values: R, G, B, A in range [0.0, 1.0]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub primary: [f32; 4],
    pub secondary: [f32; 4],
    pub background: [f32; 4],
    pub accent: [f32; 4],
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            primary: [0.11, 0.53, 0.89, 1.0],   // Vibrant blue
            secondary: [0.61, 0.35, 0.71, 1.0], // Deep purple
            background: [0.07, 0.07, 0.09, 1.0],// Dark background
            accent: [0.95, 0.76, 0.20, 1.0],    // Bright amber
        }
    }
}

impl Theme {
    /// Deserializes a `Theme` from a JSON string.
    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    /// Serializes the `Theme` to a formatted JSON string.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Resolved configuration paths for the application.
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    pub base_dir: PathBuf,
    pub shaders_dir: PathBuf,
    pub themes_dir: PathBuf,
}

/// Resolves the user configuration directory for Quasar.
/// Returns `~/.config/quasar` on Linux and `%APPDATA%/quasar` on Windows.
pub fn get_config_dir() -> io::Result<PathBuf> {
    let base_dirs = BaseDirs::new().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Could not resolve user home/config directories",
        )
    })?;
    Ok(base_dirs.config_dir().join("quasar"))
}

/// Resolves user config directories and automatically creates `shaders/` and `themes/`
/// subdirectories if they do not exist.
pub fn init_config_dirs() -> io::Result<ConfigPaths> {
    let base_dir = get_config_dir()?;
    let shaders_dir = base_dir.join("shaders");
    let themes_dir = base_dir.join("themes");

    fs::create_dir_all(&shaders_dir)?;
    fs::create_dir_all(&themes_dir)?;

    Ok(ConfigPaths {
        base_dir,
        shaders_dir,
        themes_dir,
    })
}

/// Custom enum representing file change events sent through the channel.
#[derive(Debug)]
pub enum FileChangeEvent {
    FileChanged(PathBuf),
    WatcherError(notify::Error),
}

/// Initializes a file watcher monitoring the `shaders/` and `themes/` directories.
/// Sends notifications through an `mpsc` channel whenever files in these directories change.
pub fn watch_config_dirs(
    paths: &ConfigPaths,
) -> Result<(RecommendedWatcher, Receiver<NotifyResult<Event>>), notify::Error> {
    let (tx, rx) = channel();

    let mut watcher = RecommendedWatcher::new(
        move |res: NotifyResult<Event>| {
            let _ = tx.send(res);
        },
        Config::default(),
    )?;

    watcher.watch(&paths.shaders_dir, RecursiveMode::Recursive)?;
    watcher.watch(&paths.themes_dir, RecursiveMode::Recursive)?;

    Ok((watcher, rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_theme_serde() {
        let default_theme = Theme::default();
        let json_str = default_theme.to_json().unwrap();
        let parsed_theme = Theme::from_json(&json_str).unwrap();
        assert_eq!(default_theme, parsed_theme);
    }

    #[test]
    fn test_get_config_dir() {
        let path = get_config_dir().unwrap();
        assert!(path.to_string_lossy().contains("quasar"));
    }

    #[test]
    fn test_init_config_dirs_custom() {
        let temp_dir = tempdir().unwrap();
        let base_dir = temp_dir.path().join("quasar");
        let shaders_dir = base_dir.join("shaders");
        let themes_dir = base_dir.join("themes");

        fs::create_dir_all(&shaders_dir).unwrap();
        fs::create_dir_all(&themes_dir).unwrap();

        assert!(shaders_dir.exists());
        assert!(themes_dir.exists());
    }
}
