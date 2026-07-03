use super::{Config, load_config, save_config_at};
use std::io;
use std::path::Path;

pub fn set_ignored_directories(folders: &[String]) -> io::Result<()> {
    if folders.is_empty() {
        println!("Usage: rat cfg ignore <list of folders separated by space>");
        return Ok(());
    }
    set_ignored_directories_at(&super::config_path(), folders)
}

pub fn unset_ignored_directories(folders: &[String], all: bool) -> io::Result<()> {
    if !all && folders.is_empty() {
        println!("Usage: rat cfg heed <list of folders separated by space>");
        return Ok(());
    }
    unset_ignored_directories_at(&super::config_path(), folders, all)
}

fn set_ignored_directories_at(path: &Path, folders: &[String]) -> io::Result<()> {
    let mut config = load_config(path);
    apply_ignore(&mut config, folders);
    save_config_at(path, &config)
}

fn unset_ignored_directories_at(path: &Path, folders: &[String], all: bool) -> io::Result<()> {
    let mut config = load_config(path);
    if all {
        config.ignored_folders = None;
        return save_config_at(path, &config);
    }

    if apply_heed(&mut config, folders) {
        save_config_at(path, &config)
    } else {
        println!("No changes detected. Config file not saved.");
        Ok(())
    }
}

fn apply_ignore(config: &mut Config, folders: &[String]) {
    let existing = config.ignored_folders.get_or_insert_with(Vec::new);
    for folder in folders {
        if existing.contains(folder) {
            println!("{folder} is already in the ignored folders list, skipping.");
            continue;
        }
        existing.push(folder.clone());
        println!("{folder} added to permanently ignored folders list");
    }
}

/// Returns whether any folder was actually removed.
fn apply_heed(config: &mut Config, folders: &[String]) -> bool {
    let Some(existing) = config.ignored_folders.as_mut() else {
        println!("That list is already empty");
        return false;
    };

    let mut changed = false;
    for folder in folders {
        if let Some(pos) = existing.iter().position(|f| f == folder) {
            existing.remove(pos);
            println!("{folder} removed from permanently ignored folders list");
            changed = true;
        } else {
            println!("{folder} isn't present in ignored folders list, skipping.");
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_ignore_creates_list_when_none_exists() {
        let mut config = Config::default();
        apply_ignore(&mut config, &[".git".to_string()]);
        assert_eq!(config.ignored_folders, Some(vec![".git".to_string()]));
    }

    #[test]
    fn apply_ignore_skips_duplicates() {
        let mut config = Config {
            ignored_folders: Some(vec![".git".to_string()]),
            ..Default::default()
        };
        apply_ignore(&mut config, &[".git".to_string(), ".idea".to_string()]);
        assert_eq!(
            config.ignored_folders,
            Some(vec![".git".to_string(), ".idea".to_string()])
        );
    }

    #[test]
    fn apply_heed_on_empty_list_reports_no_changes() {
        let mut config = Config::default();
        assert!(!apply_heed(&mut config, &[".git".to_string()]));
    }

    #[test]
    fn apply_heed_removes_present_folder() {
        let mut config = Config {
            ignored_folders: Some(vec![".git".to_string(), ".idea".to_string()]),
            ..Default::default()
        };
        assert!(apply_heed(&mut config, &[".git".to_string()]));
        assert_eq!(config.ignored_folders, Some(vec![".idea".to_string()]));
    }

    #[test]
    fn apply_heed_reports_unchanged_for_absent_folder() {
        let mut config = Config {
            ignored_folders: Some(vec![".idea".to_string()]),
            ..Default::default()
        };
        assert!(!apply_heed(&mut config, &[".vscode".to_string()]));
        assert_eq!(config.ignored_folders, Some(vec![".idea".to_string()]));
    }

    #[test]
    fn unset_all_clears_list_regardless_of_folders_argument() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                ignored_folders: Some(vec![".git".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();

        unset_ignored_directories_at(&path, &[], true).unwrap();

        assert_eq!(load_config(&path).ignored_folders, None);
    }
}
