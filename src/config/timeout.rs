use super::{load_config, save_config_at};
use std::io;
use std::path::Path;

pub fn set_timeout(duration: &str) -> io::Result<()> {
    if duration.is_empty() {
        println!("Forgot to add duration in `rat cfg to`?");
        return Ok(());
    }
    set_timeout_at(&super::config_path(), duration)
}

pub fn unset_timeout() -> io::Result<()> {
    unset_timeout_at(&super::config_path())
}

/// Mirrors the C# `int.TryParse` behavior: an unparseable duration is treated
/// as 0 rather than rejected, and 0 means "no timeout" (`None`).
fn parsed_timeout(duration: &str) -> Option<i32> {
    let value: i32 = duration.parse().unwrap_or(0);
    if value == 0 { None } else { Some(value) }
}

fn set_timeout_at(path: &Path, duration: &str) -> io::Result<()> {
    let mut config = load_config(path);
    config.time_out = parsed_timeout(duration);
    save_config_at(path, &config)
}

fn unset_timeout_at(path: &Path) -> io::Result<()> {
    let mut config = load_config(path);
    config.time_out = None;
    save_config_at(path, &config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn parsed_timeout_valid_duration() {
        assert_eq!(parsed_timeout("30"), Some(30));
    }

    #[test]
    fn parsed_timeout_zero_means_disabled() {
        assert_eq!(parsed_timeout("0"), None);
    }

    #[test]
    fn parsed_timeout_non_numeric_falls_back_to_disabled() {
        assert_eq!(parsed_timeout("banana"), None);
    }

    #[test]
    fn set_timeout_persists_duration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");

        set_timeout_at(&path, "45").unwrap();

        assert_eq!(load_config(&path).time_out, Some(45));
    }

    #[test]
    fn set_timeout_zero_clears_existing_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                time_out: Some(45),
                ..Default::default()
            },
        )
        .unwrap();

        set_timeout_at(&path, "0").unwrap();

        assert_eq!(load_config(&path).time_out, None);
    }

    #[test]
    fn unset_timeout_clears_existing_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".ratconfig");
        save_config_at(
            &path,
            &Config {
                time_out: Some(45),
                ..Default::default()
            },
        )
        .unwrap();

        unset_timeout_at(&path).unwrap();

        assert_eq!(load_config(&path).time_out, None);
    }
}
