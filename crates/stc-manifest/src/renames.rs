//! `stc.renames.toml`: the committed tracked-rename history (CLI §3.1).
//!
//! The file is optional; a missing file is an empty history. Entries are
//! stored oldest first and are rewritten in that order. Builds merge the
//! history with CLI `--rename` entries, compose chains oldest first, and
//! reject duplicate mappings and cycles (`STC-E-PROJECT-012/013`).

use std::collections::BTreeMap;
use std::path::Path;

use toml_edit::{ArrayOfTables, DocumentMut, Item, Table};

use crate::codes::RENAME_CONFLICT;
use crate::codes::RENAME_INVALID;

/// One tracked rename/move entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameEntry {
    /// Original key or field path.
    pub old: String,
    /// New key or field path.
    pub new: String,
}

impl RenameEntry {
    /// Creates an entry.
    #[must_use]
    pub fn new(old: impl Into<String>, new: impl Into<String>) -> Self {
        Self {
            old: old.into(),
            new: new.into(),
        }
    }
}

/// The committed rename history, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameHistory {
    /// Entries in the order they were recorded.
    pub entries: Vec<RenameEntry>,
}

impl RenameHistory {
    /// Creates an empty history.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// A rename-history or rename-merge failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameHistoryError {
    /// Stable diagnostic code (`STC-E-PROJECT-012` or `-013`).
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

impl RenameHistoryError {
    fn schema(message: impl Into<String>) -> Self {
        Self {
            code: RENAME_INVALID,
            message: message.into(),
        }
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self {
            code: RENAME_CONFLICT,
            message: message.into(),
        }
    }
}

/// Reads and validates `stc.renames.toml`.
///
/// A missing file is an empty history.
///
/// # Errors
///
/// Returns `STC-E-PROJECT-012` for unreadable, malformed, or schema-invalid
/// documents.
pub fn load_rename_history(path: &Path) -> Result<RenameHistory, RenameHistoryError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RenameHistory::new());
        }
        Err(error) => {
            return Err(RenameHistoryError::schema(format!(
                "cannot read `{}`: {error}",
                path.display()
            )));
        }
    };
    parse_rename_history(&text).map_err(|error| {
        RenameHistoryError::schema(format!("`{}`: {}", path.display(), error.message))
    })
}

/// Parses and validates a rename-history document.
///
/// # Errors
///
/// Returns `STC-E-PROJECT-012` for malformed TOML, unknown keys, a missing or
/// unsupported schema, or an invalid entry.
pub fn parse_rename_history(text: &str) -> Result<RenameHistory, RenameHistoryError> {
    let document: DocumentMut = text
        .parse()
        .map_err(|error| RenameHistoryError::schema(format!("invalid TOML: {error}")))?;
    let root = document.as_table();
    for (key, _) in root.iter() {
        if key != "schema" && key != "rename" {
            return Err(RenameHistoryError::schema(format!(
                "unknown key `{key}` (expected `schema` or `[[rename]]`)"
            )));
        }
    }
    match root.get("schema") {
        Some(Item::Value(value)) if value.as_integer() == Some(1) => {}
        Some(_) => {
            return Err(RenameHistoryError::schema("`schema` must be the integer 1"));
        }
        None => return Err(RenameHistoryError::schema("missing `schema = 1`")),
    }
    let mut history = RenameHistory::new();
    let Some(item) = root.get("rename") else {
        return Ok(history);
    };
    let Some(entries) = item.as_array_of_tables() else {
        return Err(RenameHistoryError::schema(
            "`rename` must be an array of tables (`[[rename]]`)",
        ));
    };
    for (index, table) in entries.iter().enumerate() {
        history.entries.push(parse_entry(table, index)?);
    }
    Ok(history)
}

fn parse_entry(table: &Table, index: usize) -> Result<RenameEntry, RenameHistoryError> {
    for (key, _) in table.iter() {
        if key != "old" && key != "new" {
            return Err(RenameHistoryError::schema(format!(
                "rename entry {index} has unknown key `{key}` (expected `old` and `new`)"
            )));
        }
    }
    let old = string_value(table, "old", index)?;
    let new = string_value(table, "new", index)?;
    Ok(RenameEntry::new(old, new))
}

fn string_value(table: &Table, key: &str, index: usize) -> Result<String, RenameHistoryError> {
    let Some(value) = table.get(key) else {
        return Err(RenameHistoryError::schema(format!(
            "rename entry {index} is missing `{key}`"
        )));
    };
    let Some(text) = value.as_str() else {
        return Err(RenameHistoryError::schema(format!(
            "rename entry {index}.{key} must be a string"
        )));
    };
    if text.is_empty() {
        return Err(RenameHistoryError::schema(format!(
            "rename entry {index}.{key} must not be empty"
        )));
    }
    Ok(text.to_owned())
}

/// Renders the canonical history text, oldest entry first.
#[must_use]
pub fn render_rename_history(history: &RenameHistory) -> String {
    let mut document = DocumentMut::new();
    document["schema"] = toml_edit::value(1_i64);
    let mut rename = ArrayOfTables::new();
    for entry in &history.entries {
        let mut table = Table::new();
        table.insert("old", toml_edit::value(entry.old.as_str()));
        table.insert("new", toml_edit::value(entry.new.as_str()));
        rename.push(table);
    }
    document["rename"] = Item::ArrayOfTables(rename);
    let mut rendered = document.to_string();
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

/// Appends one entry, rejecting an exact duplicate.
///
/// # Errors
///
/// Returns `STC-E-PROJECT-013` when the exact mapping is already recorded.
pub fn append_rename(
    history: &mut RenameHistory,
    old: &str,
    new: &str,
) -> Result<(), RenameHistoryError> {
    if history
        .entries
        .iter()
        .any(|entry| entry.old.eq_ignore_ascii_case(old) && entry.new.eq_ignore_ascii_case(new))
    {
        return Err(RenameHistoryError::conflict(format!(
            "rename `{old}` -> `{new}` is already recorded"
        )));
    }
    history.entries.push(RenameEntry::new(old, new));
    Ok(())
}

/// Merges the committed history with CLI entries and composes chains.
///
/// History entries come first (oldest first), followed by CLI entries in
/// command order. A duplicate `old` with a different `new`, or a chain that
/// revisits a key, is `STC-E-PROJECT-013`.
///
/// # Errors
///
/// Returns `STC-E-PROJECT-013` for a conflict or a cycle.
pub fn merge_renames(
    history: &[RenameEntry],
    cli: &[(String, String)],
) -> Result<Vec<(String, String)>, RenameHistoryError> {
    struct Mapping {
        old: String,
        new: String,
        new_key: String,
    }
    let mut order: Vec<String> = Vec::new();
    let mut mappings: BTreeMap<String, Mapping> = BTreeMap::new();
    let mut insert = |old: &str, new: &str| -> Result<(), RenameHistoryError> {
        let old_key = old.to_ascii_uppercase();
        let new_key = new.to_ascii_uppercase();
        if let Some(existing) = mappings.get(&old_key) {
            if existing.new_key == new_key {
                return Ok(());
            }
            return Err(RenameHistoryError::conflict(format!(
                "rename `{}` is recorded as both `{}` and `{new}`",
                existing.old, existing.new
            )));
        }
        order.push(old_key.clone());
        mappings.insert(
            old_key,
            Mapping {
                old: old.to_owned(),
                new: new.to_owned(),
                new_key,
            },
        );
        Ok(())
    };
    for entry in history {
        insert(&entry.old, &entry.new)?;
    }
    for (old, new) in cli {
        insert(old, new)?;
    }

    let mut resolved = Vec::with_capacity(order.len());
    for key in &order {
        let mut visited = std::collections::BTreeSet::new();
        let mut current = key.clone();
        let mut final_new = None;
        loop {
            if !visited.insert(current.clone()) {
                return Err(RenameHistoryError::conflict(format!(
                    "rename chain revisits `{}`",
                    mappings
                        .get(&current)
                        .map_or(current.as_str(), |mapping| mapping.old.as_str())
                )));
            }
            let Some(mapping) = mappings.get(&current) else {
                break;
            };
            if mapping.new_key == current {
                return Err(RenameHistoryError::conflict(format!(
                    "rename `{}` targets itself",
                    mapping.old
                )));
            }
            final_new = Some(mapping.new.clone());
            current.clone_from(&mapping.new_key);
        }
        let Some(final_new) = final_new else {
            continue;
        };
        let old = mappings
            .get(key)
            .map_or_else(|| key.clone(), |mapping| mapping.old.clone());
        resolved.push((old, final_new));
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_entries_in_order() {
        let history = parse_rename_history(
            "schema = 1\n\n[[rename]]\nold = \"Main.Speed\"\nnew = \"Main.Rate\"\n\n\
             [[rename]]\nold = \"Main.Rate\"\nnew = \"Main.Velocity\"\n",
        )
        .expect("history");
        assert_eq!(
            history.entries,
            vec![
                RenameEntry::new("Main.Speed", "Main.Rate"),
                RenameEntry::new("Main.Rate", "Main.Velocity")
            ]
        );
    }

    #[test]
    fn missing_file_is_empty() {
        let history =
            load_rename_history(Path::new("no-such-stc-renames.toml")).expect("missing is empty");
        assert!(history.entries.is_empty());
    }

    #[test]
    fn rejects_unknown_keys_and_schemas() {
        for text in [
            "schema = 2\n",
            "schema = 1\nother = 3\n",
            "schema = 1\n\n[[rename]]\nold = \"A\"\nnew = \"\"\n",
            "schema = 1\n\n[[rename]]\nold = \"A\"\nnew = \"B\"\nextra = 1\n",
            "[[rename]]\nold = \"A\"\nnew = \"B\"\n",
        ] {
            let error = parse_rename_history(text).expect_err(text);
            assert_eq!(error.code, RENAME_INVALID, "{text}");
        }
    }

    #[test]
    fn renders_canonical_history() {
        let history = RenameHistory {
            entries: vec![
                RenameEntry::new("Main.Speed", "Main.Rate"),
                RenameEntry::new("Main.Rate", "Main.Velocity"),
            ],
        };
        let text = render_rename_history(&history);
        assert_eq!(parse_rename_history(&text).unwrap(), history);
    }

    #[test]
    fn merges_and_composes_chains_oldest_first() {
        let history = vec![
            RenameEntry::new("Main.A", "Main.B"),
            RenameEntry::new("Main.B", "Main.C"),
        ];
        let merged = merge_renames(&history, &[]).expect("merge");
        assert_eq!(
            merged,
            vec![
                ("Main.A".into(), "Main.C".into()),
                ("Main.B".into(), "Main.C".into())
            ]
        );

        let merged = merge_renames(&history, &[("Main.C".into(), "Main.D".into())]).expect("merge");
        assert_eq!(
            merged,
            vec![
                ("Main.A".into(), "Main.D".into()),
                ("Main.B".into(), "Main.D".into()),
                ("Main.C".into(), "Main.D".into())
            ]
        );
    }

    #[test]
    fn rejects_conflicts_and_cycles() {
        let history = vec![RenameEntry::new("Main.A", "Main.B")];
        let error =
            merge_renames(&history, &[("main.a".into(), "Main.C".into())]).expect_err("conflict");
        assert_eq!(error.code, RENAME_CONFLICT);

        let error = merge_renames(
            &[RenameEntry::new("A", "B"), RenameEntry::new("B", "A")],
            &[],
        )
        .expect_err("cycle");
        assert_eq!(error.code, RENAME_CONFLICT);

        let error = merge_renames(&[RenameEntry::new("A", "A")], &[]).expect_err("self");
        assert_eq!(error.code, RENAME_CONFLICT);
    }
}
