mod commands;
mod ignore;
mod print;
mod timeout;
mod workdir;

pub use commands::evaluate;
pub use ignore::{set_ignored_directories, unset_ignored_directories};
pub use print::{print_config, print_config_path};
pub use timeout::{set_timeout, unset_timeout};
pub use workdir::{set_working_directory, unset_working_directory};

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(rename = "defaultFolder", skip_serializing_if = "Option::is_none")]
    pub default_folder: Option<String>,
    #[serde(rename = "ignoredFolders", skip_serializing_if = "Option::is_none")]
    pub ignored_folders: Option<Vec<String>>,
    #[serde(rename = "timeOut", skip_serializing_if = "Option::is_none")]
    pub time_out: Option<i32>,
}

/// `~/.ratconfig`. Note this is a fresh path (the C# predecessor `ath` used
/// `~/.athconfig`) - the two tools do not share config files.
pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ratconfig")
}

/// Loads config from `path`. Missing file or malformed JSON both fall back to
/// `Config::default()` rather than erroring - the original threw an uncaught
/// exception on malformed JSON; failing open on a corrupt config is friendlier
/// for a CLI tool than crashing on every invocation until the user finds and
/// fixes/deletes the file by hand.
pub fn load_config(path: &Path) -> Config {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

pub fn get_config() -> Config {
    load_config(&config_path())
}

/// Writes `config` to `path` via write-then-rename rather than a direct
/// `fs::write`, so a process killed mid-write (Ctrl-C, crash, ...) can't
/// leave `path` truncated/corrupted - the rename only replaces the old
/// file once the new contents are fully and successfully written.
/// `fs::rename` onto an existing path is atomic on both POSIX (`rename(2)`)
/// and Windows (`MoveFileExW` with replace-existing, which is what
/// `std::fs::rename` uses there).
pub fn save_config_at(path: &Path, config: &Config) -> io::Result<()> {
    let json = serde_json::to_string_pretty(config)?;

    let temp_path = temp_path_for(path);
    std::fs::write(&temp_path, json)?;
    std::fs::rename(&temp_path, path)?;

    println!("Config file updated successfully!");
    Ok(())
}

fn temp_path_for(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(".ratconfig");
    path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_config_missing_file_returns_default() {
        let path = std::env::temp_dir().join("rat-test-does-not-exist.json");
        assert_eq!(load_config(&path), Config::default());
    }

    #[test]
    fn load_config_malformed_json_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_config(&path), Config::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        let config = Config {
            default_folder: Some("/home/me/projects".into()),
            ignored_folders: Some(vec![".git".into(), ".idea".into()]),
            time_out: Some(30),
        };

        save_config_at(&path, &config).unwrap();
        assert_eq!(load_config(&path), config);
    }

    #[test]
    fn save_config_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");

        save_config_at(&path, &Config::default()).unwrap();

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(
            entries,
            vec![path],
            "expected only the config file itself, no leftover temp file"
        );
    }

    #[test]
    fn saved_json_uses_camel_case_field_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        let config = Config {
            default_folder: Some("/tmp".into()),
            ignored_folders: None,
            time_out: Some(5),
        };

        save_config_at(&path, &config).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("\"defaultFolder\""));
        assert!(written.contains("\"timeOut\""));
        assert!(!written.contains("ignoredFolders"));
    }
}
