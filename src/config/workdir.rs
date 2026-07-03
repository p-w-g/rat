use super::{load_config, save_config_at};
use std::io;
use std::path::Path;

pub fn set_working_directory() -> io::Result<()> {
    let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
    set_working_directory_at(&super::config_path(), cwd)
}

pub fn unset_working_directory() -> io::Result<()> {
    unset_working_directory_at(&super::config_path())
}

fn set_working_directory_at(path: &Path, cwd: String) -> io::Result<()> {
    let mut config = load_config(path);
    config.default_folder = Some(cwd);
    save_config_at(path, &config)
}

fn unset_working_directory_at(path: &Path) -> io::Result<()> {
    let mut config = load_config(path);
    config.default_folder = None;
    save_config_at(path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn set_working_directory_writes_current_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");

        set_working_directory_at(&path, "/some/project".into()).unwrap();

        assert_eq!(
            load_config(&path).default_folder,
            Some("/some/project".to_string())
        );
    }

    #[test]
    fn unset_working_directory_clears_existing_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                default_folder: Some("/some/project".into()),
                ..Default::default()
            },
        )
        .unwrap();

        unset_working_directory_at(&path).unwrap();

        assert_eq!(load_config(&path).default_folder, None);
    }
}
