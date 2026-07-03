use std::io;
use std::path::{Path, PathBuf};

/// Resolves the working directory: `--local` always wins (forces CWD even
/// if a default folder is configured), otherwise the configured default
/// folder wins, falling back to CWD if neither applies.
///
/// Takes already-extracted primitives rather than `ParsedArgs`/`Config`
/// directly so this stays a leaf module with no dependency on `cli`'s own
/// parser output or on the `config` module's types.
pub fn assume_working_directory(local_flag: bool, default_folder: Option<&str>) -> PathBuf {
    let cwd = || std::env::current_dir().expect("current directory should be accessible");

    if local_flag {
        return cwd();
    }

    match default_folder {
        Some(folder) => PathBuf::from(folder),
        None => cwd(),
    }
}

/// Lists the immediate subdirectories of `working_directory`, filtered by
/// ignored/skip/only. Mirrors DirParsing.cs's `GetAvailableDirectories`.
pub fn available_directories(
    working_directory: &Path,
    ignored_folders: Option<&[String]>,
    skip: Option<&[String]>,
    only: Option<&[String]>,
) -> io::Result<Vec<PathBuf>> {
    let all_directories: Vec<PathBuf> = std::fs::read_dir(working_directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();

    Ok(filter_available_directories(
        all_directories,
        ignored_folders,
        skip,
        only,
    ))
}

/// Pure filtering logic, separated from the real directory listing above so
/// it's testable against fabricated paths with no filesystem involved.
///
/// `only` and `skip` are mutually exclusive: if `only` is present, `skip` is
/// not applied at all, even if both were supplied. That's the original's
/// behavior - preserved as-is here, not something to "fix".
fn filter_available_directories(
    all_directories: Vec<PathBuf>,
    ignored_folders: Option<&[String]>,
    skip: Option<&[String]>,
    only: Option<&[String]>,
) -> Vec<PathBuf> {
    let mut directories = all_directories;

    if let Some(ignored) = ignored_folders {
        directories = remove_target_directories(directories, ignored);
    }

    if let Some(skip) = skip
        && only.is_none()
    {
        directories = remove_target_directories(directories, skip);
    }

    if let Some(only) = only {
        directories = select_target_directories(directories, only);
    }

    directories
}

/// Matches by substring on the full path, same as the original's
/// `dir.Contains(path)` - not an exact folder-name comparison.
fn remove_target_directories(directories: Vec<PathBuf>, paths: &[String]) -> Vec<PathBuf> {
    directories
        .into_iter()
        .filter(|dir| {
            let dir_str = dir.to_string_lossy();
            !paths.iter().any(|p| dir_str.contains(p.as_str()))
        })
        .collect()
}

fn select_target_directories(directories: Vec<PathBuf>, paths: &[String]) -> Vec<PathBuf> {
    directories
        .into_iter()
        .filter(|dir| {
            let dir_str = dir.to_string_lossy();
            paths.iter().any(|p| dir_str.contains(p.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn local_flag_forces_cwd_even_with_default_folder() {
        let result = assume_working_directory(true, Some("/configured/default"));
        assert_eq!(result, std::env::current_dir().unwrap());
    }

    #[test]
    fn default_folder_wins_when_local_not_set() {
        let result = assume_working_directory(false, Some("/configured/default"));
        assert_eq!(result, PathBuf::from("/configured/default"));
    }

    #[test]
    fn falls_back_to_cwd_when_nothing_configured() {
        let result = assume_working_directory(false, None);
        assert_eq!(result, std::env::current_dir().unwrap());
    }

    #[test]
    fn no_filters_passes_through_unchanged() {
        let all = paths(&["/w/api", "/w/web", "/w/.git"]);
        let result = filter_available_directories(all.clone(), None, None, None);
        assert_eq!(result, all);
    }

    #[test]
    fn ignored_folders_are_removed() {
        let all = paths(&["/w/api", "/w/web", "/w/.git"]);
        let result = filter_available_directories(all, Some(&strings(&[".git"])), None, None);
        assert_eq!(result, paths(&["/w/api", "/w/web"]));
    }

    #[test]
    fn skip_is_applied_when_only_absent() {
        let all = paths(&["/w/api", "/w/web", "/w/docs"]);
        let result = filter_available_directories(all, None, Some(&strings(&["web"])), None);
        assert_eq!(result, paths(&["/w/api", "/w/docs"]));
    }

    #[test]
    fn skip_is_ignored_entirely_when_only_present() {
        let all = paths(&["/w/api", "/w/web", "/w/docs"]);
        let result = filter_available_directories(
            all,
            None,
            Some(&strings(&["api"])),
            Some(&strings(&["web"])),
        );
        // "api" skip has no effect at all; only "web" survives via `only`.
        assert_eq!(result, paths(&["/w/web"]));
    }

    #[test]
    fn only_selects_matching_directories() {
        let all = paths(&["/w/api", "/w/web", "/w/docs"]);
        let result = filter_available_directories(all, None, None, Some(&strings(&["api", "web"])));
        assert_eq!(result, paths(&["/w/api", "/w/web"]));
    }

    #[test]
    fn all_three_filters_compose() {
        let all = paths(&["/w/api", "/w/web", "/w/.git", "/w/docs"]);
        let result = filter_available_directories(
            all,
            Some(&strings(&[".git"])),
            None,
            Some(&strings(&["api", "web", "docs"])),
        );
        assert_eq!(result, paths(&["/w/api", "/w/web", "/w/docs"]));
    }

    #[test]
    fn available_directories_lists_real_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("api")).unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        std::fs::write(dir.path().join("not-a-dir.txt"), "x").unwrap();

        let mut result = available_directories(dir.path(), None, None, None)
            .unwrap()
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        result.sort();

        assert_eq!(result, vec!["api".to_string(), "web".to_string()]);
    }

    #[test]
    fn available_directories_applies_ignore_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("api")).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let result = available_directories(dir.path(), Some(&strings(&[".git"])), None, None)
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_name().unwrap().to_string_lossy(), "api");
    }
}
