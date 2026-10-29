//! Host filesystem path semantics.
//!
//! Windows paths are compared case-insensitively and `.st` is matched
//! case-insensitively; Linux paths are case-sensitive (CLI and Manifest §2.3).
//! This module is the single operating-system conditional in the manifest
//! crate (D-201); the model and graph code call these functions instead of
//! branching on the host.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

/// Whether filename comparison ignores ASCII case on this host.
pub(crate) const CASE_INSENSITIVE_PATHS: bool = cfg!(windows);

/// Compares two path spellings using host semantics.
pub(crate) fn paths_equal(left: &str, right: &str) -> bool {
    if CASE_INSENSITIVE_PATHS {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

/// Compares two path components using host semantics.
pub(crate) fn components_equal(left: Component<'_>, right: Component<'_>) -> bool {
    let left = left.as_os_str().to_string_lossy();
    let right = right.as_os_str().to_string_lossy();
    paths_equal(&left, &right)
}

/// Returns whether `name` names the build output directory.
pub(crate) fn is_target_dir(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    if CASE_INSENSITIVE_PATHS {
        name.eq_ignore_ascii_case("target")
    } else {
        name == "target"
    }
}

/// Returns whether `path` matches one of the manifest's source exclusions.
pub(crate) fn is_excluded(path: &Path, exclusions: &[PathBuf]) -> bool {
    exclusions.iter().any(|exclusion| {
        if path == exclusion {
            return true;
        }
        CASE_INSENSITIVE_PATHS
            && path
                .to_string_lossy()
                .eq_ignore_ascii_case(&exclusion.to_string_lossy())
    })
}

/// Returns whether `path` has the `.st` source extension under host semantics.
pub(crate) fn has_st_extension(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };
    if CASE_INSENSITIVE_PATHS {
        extension.eq_ignore_ascii_case("st")
    } else {
        extension == "st"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_and_target_semantics_follow_the_host() {
        assert!(has_st_extension(Path::new("main.st")));
        assert!(!has_st_extension(Path::new("main.txt")));
        assert!(is_target_dir(OsStr::new("target")));
        assert!(!is_target_dir(OsStr::new("targets")));
        #[cfg(windows)]
        {
            assert!(has_st_extension(Path::new("MAIN.ST")));
            assert!(is_target_dir(OsStr::new("TARGET")));
        }
        #[cfg(not(windows))]
        {
            assert!(!has_st_extension(Path::new("MAIN.ST")));
            assert!(!is_target_dir(OsStr::new("TARGET")));
        }
    }

    #[test]
    fn path_comparison_follows_the_host() {
        assert!(paths_equal("src/main.st", "src/main.st"));
        #[cfg(windows)]
        assert!(paths_equal("SRC/Main.ST", "src/main.st"));
        #[cfg(not(windows))]
        assert!(!paths_equal("SRC/Main.ST", "src/main.st"));
    }
}
