//! Deterministic ESI library: search, explicit source selection and
//! identity/revision resolution.
//!
//! Resolution is deterministic: files are visited in sorted path order, and
//! a single identity that selects descriptions with differing content is
//! reported as ambiguous rather than silently first-matched (D-263, D-280).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::model::{DeviceDescription, EsiDocument, parse_number};
use super::reader::{Limits, parse_file_with};
use crate::ethercat::diagnostics::{EngineeringDiagnostic, codes};

/// Maximum number of ESI files accepted by one library.
pub const MAX_LIBRARY_FILES: usize = 4096;

/// Maximum directory nesting the library walk follows.
pub const MAX_LIBRARY_DEPTH: usize = 16;

const OPERATION: &str = "esi.list";

fn unreadable(source: &Path, observed: impl std::fmt::Display) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_SOURCE_UNREADABLE,
        OPERATION,
        "check the ESI library path and permissions",
    )
    .with_source(source.display().to_string())
    .with_observed(observed.to_string())
}

fn limit_exceeded(source: &Path, observed: String, limit: usize) -> EngineeringDiagnostic {
    EngineeringDiagnostic::new(
        codes::ESI_LIMIT_EXCEEDED,
        OPERATION,
        "reduce the library or raise the documented limit deliberately",
    )
    .with_source(source.display().to_string())
    .with_expected(format!("at most {limit}"))
    .with_observed(observed)
}

/// One device description inside the library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibraryEntry {
    /// Source file.
    pub source: PathBuf,
    /// Declaration index inside the source document.
    pub device_index: usize,
    /// Vendor id declared by the document.
    pub vendor_id: u32,
    /// Vendor name declared by the document.
    pub vendor_name: String,
    /// Product code.
    pub product_code: u32,
    /// Revision number.
    pub revision: u32,
    /// Device name.
    pub name: String,
    /// Device type name.
    pub type_name: String,
    /// Content digest of the accepted description.
    pub digest: String,
}

/// A device resolved by exact identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedDevice {
    /// Library entry the identity resolved to.
    pub entry: LibraryEntry,
    /// The accepted description.
    pub device: DeviceDescription,
    /// Whether the caller pinned the revision.
    pub pinned: bool,
    /// Constructs of the source document the accepted subset does not model.
    pub unsupported: Vec<String>,
}

/// A loaded ESI library.
#[derive(Debug)]
pub struct EsiLibrary {
    root: PathBuf,
    documents: Vec<EsiDocument>,
    problems: Vec<EngineeringDiagnostic>,
}

impl EsiLibrary {
    /// Loads every `.xml` file under `root` with the documented limits.
    ///
    /// # Errors
    ///
    /// Returns an `STC-E-ESI-*` diagnostic when the root cannot be read or a
    /// resource limit is exceeded. A single unreadable or unsupported file is
    /// recorded in [`Self::problems`] and does not hide the rest of the library.
    pub fn load(root: &Path) -> Result<Self, EngineeringDiagnostic> {
        Self::load_with(root, &Limits::default())
    }

    /// Loads a library with explicit limits.
    ///
    /// # Errors
    ///
    /// Returns an `STC-E-ESI-*` diagnostic as [`Self::load`] does.
    pub fn load_with(root: &Path, limits: &Limits) -> Result<Self, EngineeringDiagnostic> {
        let canonical = std::fs::canonicalize(root).map_err(|error| unreadable(root, error))?;
        let mut files = Vec::new();
        if canonical.is_file() {
            files.push(canonical.clone());
        } else {
            collect_files(&canonical, &canonical, &mut files, 0)?;
        }
        let mut documents = Vec::with_capacity(files.len());
        let mut problems = Vec::new();
        for file in files {
            match parse_file_with(&file, limits) {
                Ok(document) => documents.push(document),
                Err(problem) => problems.push(problem),
            }
        }
        Ok(Self {
            root: canonical,
            documents,
            problems,
        })
    }

    /// Library root as resolved at load time.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Parsed documents in deterministic path order.
    #[must_use]
    pub fn documents(&self) -> &[EsiDocument] {
        &self.documents
    }

    /// Files that could not be accepted, with their `STC-E-ESI-*` diagnostics.
    #[must_use]
    pub fn problems(&self) -> &[EngineeringDiagnostic] {
        &self.problems
    }

    /// Every device description in deterministic path/declaration order.
    #[must_use]
    pub fn entries(&self) -> Vec<LibraryEntry> {
        let mut entries: Vec<LibraryEntry> = self
            .documents
            .iter()
            .flat_map(|document| {
                document
                    .devices
                    .iter()
                    .enumerate()
                    .map(move |(index, device)| LibraryEntry {
                        source: document.source.clone(),
                        device_index: index,
                        vendor_id: device.vendor_id,
                        vendor_name: document.vendor.name.clone(),
                        product_code: device.product_code,
                        revision: device.revision,
                        name: device.name.clone(),
                        type_name: device.type_name.clone(),
                        digest: device.digest.clone(),
                    })
            })
            .collect();
        entries.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then(left.device_index.cmp(&right.device_index))
        });
        entries
    }

    /// Case-insensitive search over vendor/type/name and identity spellings.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<LibraryEntry> {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            return Vec::new();
        }
        self.entries()
            .into_iter()
            .filter(|entry| {
                entry.vendor_name.to_lowercase().contains(&query)
                    || entry.name.to_lowercase().contains(&query)
                    || entry.type_name.to_lowercase().contains(&query)
                    || format!("#x{:08x}", entry.product_code).contains(&query)
                    || format!("#x{:08x}", entry.revision).contains(&query)
                    || entry.product_code.to_string() == query
            })
            .collect()
    }

    /// Resolves an exact identity.
    ///
    /// An absent revision selects the highest declared revision (deterministic
    /// and reported through [`ResolvedDevice::pinned`]). Within the selected
    /// revision, differing content digests are ambiguous.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-006` when one identity selects differing
    /// descriptions and `STC-E-ESI-007` when nothing matches.
    pub fn resolve(
        &self,
        vendor_id: u32,
        product_code: u32,
        revision: Option<u32>,
    ) -> Result<ResolvedDevice, EngineeringDiagnostic> {
        let mut candidates: Vec<LibraryEntry> = self
            .entries()
            .into_iter()
            .filter(|entry| {
                entry.vendor_id == vendor_id
                    && entry.product_code == product_code
                    && revision.is_none_or(|revision| entry.revision == revision)
            })
            .collect();
        if candidates.is_empty() {
            return Err(EngineeringDiagnostic::new(
                codes::ESI_IDENTITY_NOT_FOUND,
                "esi.resolve",
                "import the device description or correct the identity",
            )
            .with_identity(vendor_id, product_code, revision)
            .with_expected(identity_text(vendor_id, product_code, revision)));
        }
        let pinned = revision.is_some();
        if revision.is_none() {
            let highest = candidates
                .iter()
                .map(|entry| entry.revision)
                .max()
                .unwrap_or(0);
            candidates.retain(|entry| entry.revision == highest);
        }

        let mut by_digest: BTreeMap<&str, Vec<&LibraryEntry>> = BTreeMap::new();
        for entry in &candidates {
            by_digest
                .entry(entry.digest.as_str())
                .or_default()
                .push(entry);
        }
        if by_digest.len() > 1 {
            let mut observed: Vec<String> = candidates
                .iter()
                .map(|entry| format!("{}#{}", entry.source.display(), entry.device_index))
                .collect();
            observed.sort();
            return Err(EngineeringDiagnostic::new(
                codes::ESI_IDENTITY_AMBIGUOUS,
                "esi.resolve",
                "keep one description for the identity or pin an explicit revision",
            )
            .with_identity(vendor_id, product_code, revision)
            .with_expected(format!(
                "one description for {}",
                identity_text(vendor_id, product_code, revision)
            ))
            .with_observed(observed.join(", ")));
        }
        let chosen = candidates
            .iter()
            .min_by(|left, right| {
                left.source
                    .cmp(&right.source)
                    .then(left.device_index.cmp(&right.device_index))
            })
            .ok_or_else(|| {
                EngineeringDiagnostic::new(
                    codes::ESI_IDENTITY_NOT_FOUND,
                    "esi.resolve",
                    "import the device description",
                )
                .with_identity(vendor_id, product_code, revision)
            })?;
        let document = self
            .documents
            .iter()
            .find(|document| document.source == chosen.source)
            .ok_or_else(|| {
                EngineeringDiagnostic::new(
                    codes::ESI_IDENTITY_NOT_FOUND,
                    "esi.resolve",
                    "reload the ESI library",
                )
                .with_identity(vendor_id, product_code, revision)
            })?;
        let device = document.device(chosen.device_index).ok_or_else(|| {
            EngineeringDiagnostic::new(
                codes::ESI_IDENTITY_NOT_FOUND,
                "esi.resolve",
                "reload the ESI library",
            )
            .with_identity(vendor_id, product_code, revision)
        })?;
        Ok(ResolvedDevice {
            entry: chosen.clone(),
            device: device.clone(),
            pinned,
            unsupported: document.unsupported.clone(),
        })
    }

    /// Parses one explicit source, confined to the library root.
    ///
    /// # Errors
    ///
    /// Returns `STC-E-ESI-001` for a source outside the root and otherwise the
    /// reader's diagnostics.
    pub fn parse_source(&self, source: &Path) -> Result<EsiDocument, EngineeringDiagnostic> {
        let canonical = std::fs::canonicalize(source).map_err(|error| unreadable(source, error))?;
        let inside = if self.root.is_file() {
            canonical == self.root
        } else {
            canonical.starts_with(&self.root)
        };
        if !inside {
            return Err(unreadable(
                source,
                format!("outside the ESI library root `{}`", self.root.display()),
            ));
        }
        parse_file_with(&canonical, &Limits::default())
    }
}

fn identity_text(vendor_id: u32, product_code: u32, revision: Option<u32>) -> String {
    match revision {
        None => format!("vendor #x{vendor_id:08x} product #x{product_code:08x}"),
        Some(revision) => format!(
            "vendor #x{vendor_id:08x} product #x{product_code:08x} revision #x{revision:08x}"
        ),
    }
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<PathBuf>,
    depth: usize,
) -> Result<(), EngineeringDiagnostic> {
    if depth > MAX_LIBRARY_DEPTH {
        return Err(limit_exceeded(
            root,
            format!("directory depth above {}", MAX_LIBRARY_DEPTH),
            MAX_LIBRARY_DEPTH,
        ));
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(directory)
        .map_err(|error| unreadable(directory, error))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()
        .map_err(|error| unreadable(directory, error))?;
    entries.sort();
    for path in entries {
        let metadata =
            std::fs::symlink_metadata(&path).map_err(|error| unreadable(&path, error))?;
        if metadata.is_dir() {
            collect_files(root, &path, files, depth + 1)?;
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            let canonical =
                std::fs::canonicalize(&path).map_err(|error| unreadable(&path, error))?;
            if !canonical.starts_with(root) {
                continue;
            }
            files.push(canonical);
            if files.len() > MAX_LIBRARY_FILES {
                return Err(limit_exceeded(
                    root,
                    format!("more than {MAX_LIBRARY_FILES} files"),
                    MAX_LIBRARY_FILES,
                ));
            }
        }
    }
    Ok(())
}

/// Parses an identity from text (`#x00000002`, `0x2`, or decimal).
#[must_use]
pub fn parse_identity(text: &str) -> Option<u32> {
    parse_number(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_text_distinguishes_pinned_revisions() {
        assert_eq!(
            identity_text(2, 2, None),
            "vendor #x00000002 product #x00000002"
        );
        assert!(identity_text(2, 2, Some(1)).contains("revision #x00000001"));
        assert_eq!(parse_identity("#x2"), Some(2));
        assert_eq!(parse_identity("nope"), None);
    }

    #[test]
    fn missing_root_is_reported_as_unreadable() {
        let root = std::env::temp_dir().join(format!("stc-esi-missing-{}", std::process::id()));
        let error = EsiLibrary::load(&root).expect_err("missing root");
        assert_eq!(error.code, codes::ESI_SOURCE_UNREADABLE);
        assert!(error.source.is_some());
    }
}
