//! File-URI to path conversion.
//!
//! The `url` crate performs the percent-encoding and host handling. Windows
//! verbatim paths (`\\?\C:\...`, `\\?\UNC\server\share\...`), as returned by
//! `std::fs::canonicalize`, are normalized to their plain spelling first
//! because `Url::from_file_path` rejects the verbatim prefix.

use std::path::{Path, PathBuf};

use url::Url;

/// Converts an absolute file path to a `file:` URI.
///
/// Returns `None` for relative paths and paths that cannot be represented.
#[must_use]
pub fn path_to_uri(path: &Path) -> Option<Url> {
    Url::from_file_path(normalize(path)).ok()
}

/// Converts a `file:` URI to a path.
///
/// Returns `None` for non-file URIs and unparsable input.
#[must_use]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let path = Url::parse(uri).ok()?.to_file_path().ok()?;
    Some(normalize(&path))
}

/// Returns the URI spelling used as a document-store key.
///
/// A parseable URI is normalized by the `url` crate; anything else is kept
/// verbatim so callers can still report a diagnostics placeholder.
#[must_use]
pub fn normalize_uri(uri: &str) -> String {
    Url::parse(uri).map_or_else(|_| uri.to_owned(), |url| url.to_string())
}

#[cfg(windows)]
fn normalize(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(not(windows))]
fn normalize(path: &Path) -> PathBuf {
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_an_absolute_path_with_spaces_and_unicode() {
        let base = std::env::temp_dir();
        let path = base.join("stc lsp ünïcode");
        let uri = path_to_uri(&path).expect("file URI");
        assert!(uri.as_str().starts_with("file://"));
        assert_eq!(uri_to_path(uri.as_str()), Some(path));
    }

    #[test]
    fn rejects_relative_paths_and_non_file_uris() {
        assert!(path_to_uri(Path::new("relative/main.st")).is_none());
        assert!(uri_to_path("untitled:Untitled-1").is_none());
        assert!(uri_to_path("not a uri").is_none());
    }

    #[test]
    fn normalizes_uri_spelling() {
        let uri = "file:///C:/repos/a%20b/main.st";
        assert_eq!(normalize_uri(uri), uri);
        assert_eq!(normalize_uri("not a uri"), "not a uri");
    }

    #[cfg(windows)]
    #[test]
    fn strips_the_windows_verbatim_prefix() {
        let plain = Path::new(r"C:\repos\stc3\examples\hello\stc.toml");
        let verbatim = Path::new(r"\\?\C:\repos\stc3\examples\hello\stc.toml");
        assert_eq!(
            path_to_uri(verbatim).map(|uri| uri.to_string()),
            path_to_uri(plain).map(|uri| uri.to_string())
        );
        let unc = Path::new(r"\\?\UNC\server\share\main.st");
        let uri = path_to_uri(unc).expect("UNC URI");
        assert!(uri.as_str().starts_with("file://server/share/"), "{uri}");
    }
}
