use crate::cli::filter::{DirectoryMatcher, FilterExpression};
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
/// the permanently-ignored list and then by `filter`. Mirrors
/// DirParsing.cs's `GetAvailableDirectories`.
pub fn available_directories(
    working_directory: &Path,
    ignored_folders: Option<&[String]>,
    filter: &FilterExpression,
) -> io::Result<Vec<PathBuf>> {
    let all_directories: Vec<PathBuf> = std::fs::read_dir(working_directory)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();

    Ok(filter_available_directories(
        all_directories,
        ignored_folders,
        filter,
    ))
}

/// Pure filtering logic, separated from the real directory listing above so
/// it's testable against fabricated paths with no filesystem involved.
///
/// `only` and `skip` both apply when given: a directory must satisfy `only`
/// (if present) *and* not match `skip` (if present) - see
/// `DirectoryMatcher`. This is a deliberate change from `only` making `skip`
/// a no-op entirely, which was the previous (and the original C# tool's)
/// behavior.
fn filter_available_directories(
    all_directories: Vec<PathBuf>,
    ignored_folders: Option<&[String]>,
    filter: &FilterExpression,
) -> Vec<PathBuf> {
    let mut directories = all_directories;

    if let Some(ignored) = ignored_folders {
        directories = remove_ignored_directories(directories, ignored);
    }

    if !filter.is_empty() {
        let matcher = DirectoryMatcher::default();
        directories.retain(|dir| {
            let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            matcher.matches(name, filter)
        });
    }

    directories
}

/// The permanently-ignored list (`cfg ignore`) is matched by substring on
/// the full path, same as the original's `dir.Contains(path)`. This is a
/// separate, persistent, path-based exclusion list rather than a per-run
/// `--only`/`--skip` filter, so it's deliberately left out of the
/// component matcher above - `.git`/`.idea`/etc. entries are exact folder
/// names in practice, and users may reasonably ignore a path fragment
/// rather than a single name component.
fn remove_ignored_directories(directories: Vec<PathBuf>, paths: &[String]) -> Vec<PathBuf> {
    directories
        .into_iter()
        .filter(|dir| {
            let dir_str = dir.to_string_lossy();
            !paths.iter().any(|p| dir_str.contains(p.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::filter::FilterExpression;

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    fn filter(only: Option<&[&str]>, skip: Option<&[&str]>) -> FilterExpression {
        let only = only.map(strings);
        let skip = skip.map(strings);
        FilterExpression::new(only.as_deref(), skip.as_deref())
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
        let result = filter_available_directories(all.clone(), None, &FilterExpression::default());
        assert_eq!(result, all);
    }

    #[test]
    fn ignored_folders_are_removed() {
        let all = paths(&["/w/api", "/w/web", "/w/.git"]);
        let result = filter_available_directories(
            all,
            Some(&strings(&[".git"])),
            &FilterExpression::default(),
        );
        assert_eq!(result, paths(&["/w/api", "/w/web"]));
    }

    #[test]
    fn skip_is_applied_when_only_absent() {
        let all = paths(&["/w/api", "/w/web", "/w/docs"]);
        let result = filter_available_directories(all, None, &filter(None, Some(&["web"])));
        assert_eq!(result, paths(&["/w/api", "/w/docs"]));
    }

    #[test]
    fn skip_narrows_only_instead_of_being_ignored() {
        // Regression test for a deliberate behavior change: `only` used to
        // make `skip` a no-op entirely. The component matcher instead
        // applies both - a directory must satisfy `only` *and* not match
        // `skip`.
        let all = paths(&["/w/api", "/w/web", "/w/docs"]);
        let result = filter_available_directories(
            all,
            None,
            &filter(Some(&["web", "docs"]), Some(&["docs"])),
        );
        assert_eq!(result, paths(&["/w/web"]));
    }

    #[test]
    fn only_selects_matching_directories() {
        let all = paths(&["/w/api", "/w/web", "/w/docs"]);
        let result = filter_available_directories(all, None, &filter(Some(&["api", "web"]), None));
        assert_eq!(result, paths(&["/w/api", "/w/web"]));
    }

    #[test]
    fn ignore_and_only_compose() {
        let all = paths(&["/w/api", "/w/web", "/w/.git", "/w/docs"]);
        let result = filter_available_directories(
            all,
            Some(&strings(&[".git"])),
            &filter(Some(&["api", "web", "docs"]), None),
        );
        assert_eq!(result, paths(&["/w/api", "/w/web", "/w/docs"]));
    }

    #[test]
    fn only_matches_the_directorys_own_name_not_its_full_path() {
        // The parent folder ("web") must not affect matching against its
        // children - only the child directory's own name is tokenized.
        let all = paths(&["/w/web/api", "/w/web/docs"]);
        let result = filter_available_directories(all, None, &filter(Some(&["web"]), None));
        assert_eq!(result, Vec::<PathBuf>::new());
    }

    #[test]
    fn available_directories_lists_real_subdirectories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("api")).unwrap();
        std::fs::create_dir(dir.path().join("web")).unwrap();
        std::fs::write(dir.path().join("not-a-dir.txt"), "x").unwrap();

        let mut result = available_directories(dir.path(), None, &FilterExpression::default())
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

        let result = available_directories(
            dir.path(),
            Some(&strings(&[".git"])),
            &FilterExpression::default(),
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].file_name().unwrap().to_string_lossy(), "api");
    }
}
