use std::path::Path;

pub fn print_config_path() {
    println!("{}", config_path_message(&super::config_path()));
}

pub fn print_config() {
    println!("{}", config_contents_message(&super::config_path()));
}

fn config_path_message(path: &Path) -> String {
    if path.exists() {
        path.display().to_string()
    } else {
        "config file doesn't exist".to_string()
    }
}

fn config_contents_message(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|_| "config file doesn't exist".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_message_reports_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        assert_eq!(config_path_message(&path), "config file doesn't exist");
    }

    #[test]
    fn config_path_message_reports_existing_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(config_path_message(&path), path.display().to_string());
    }

    #[test]
    fn config_contents_message_reports_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        assert_eq!(config_contents_message(&path), "config file doesn't exist");
    }

    #[test]
    fn config_contents_message_returns_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        std::fs::write(&path, "{\"timeOut\":30}").unwrap();
        assert_eq!(config_contents_message(&path), "{\"timeOut\":30}");
    }
}
