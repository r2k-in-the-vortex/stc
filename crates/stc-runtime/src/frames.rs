//! Stopped-frame sidecar loading (Debug and Monitoring §16.9, D-181).
//!
//! Hook-emitting debug builds write `<name>.stc-frames.json` beside the module
//! and its metadata sidecar. The table pairs with the build identity but is not
//! part of module, manifest, descriptor, state, metadata, or ABI identity. The
//! runtime never parses DWARF or CodeView: it reads the build-time extraction
//! and validates every field before use.
//!
//! A missing, unreadable, or malformed table never fails module loading. The
//! table is optional and its absence only disables `stack.list` and
//! `frame.read` with `STC-E-DEBUG-017`.

use std::collections::BTreeMap;
use std::path::Path;

use stc_got::layout::TypeKey;
use stc_got::{GotId, Json, Target, json};

/// Maximum number of typed locals across one frame table.
pub(crate) const MAX_LOCALS: usize = 1_000_000;

/// Maximum serialized stopped-frame table size.
pub(crate) const MAX_TABLE_BYTES: u64 = 64 * 1024 * 1024;

/// Upper bound for one function's machine-code size.
const MAX_CODE_SIZE: u32 = 64 * 1024 * 1024;

/// Upper bound for one function's stack-frame size.
const MAX_FRAME_SIZE: u32 = 64 * 1024 * 1024;

/// Upper bound for a reported symbol name.
const MAX_SYMBOL_LEN: usize = 512;

/// Upper bound for a reported local name.
const MAX_NAME_LEN: usize = 128;

/// Upper bound for a canonical type key.
const MAX_TYPE_LEN: usize = 4096;

/// Upper bound for a reported source path.
const MAX_FILE_LEN: usize = 1024;

const TABLE_KEYS: [&str; 5] = ["build_id", "format", "functions", "pointer_width", "target"];
const FUNCTION_KEYS: [&str; 9] = [
    "cfa",
    "frame_size",
    "function",
    "high_pc",
    "locals",
    "low_pc",
    "return_address",
    "saved_fp",
    "symbol",
];
const LOCAL_KEYS: [&str; 5] = ["live", "location", "name", "size", "type"];
const LIVE_KEYS: [&str; 2] = ["first", "last"];

/// One typed local described by the stopped-frame table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameLocal {
    /// Normalized (uppercase) name.
    pub(crate) name: String,
    /// Canonical type key.
    pub(crate) type_key: String,
    /// Storage size in bytes.
    pub(crate) size: u32,
    /// Positive distance below the frame pointer: the value lives at `fp - offset`.
    pub(crate) offset: u32,
    /// First checkpoint ID at which the storage is live.
    pub(crate) live_first: u32,
    /// Last checkpoint ID at which the storage is live.
    pub(crate) live_last: u32,
}

impl FrameLocal {
    /// Returns the storage address for a frame pointer, if it stays in range.
    #[must_use]
    pub(crate) fn address(&self, fp: usize) -> Option<usize> {
        let offset = usize::try_from(self.offset).ok()?;
        let start = fp.checked_sub(offset)?;
        let end = start.checked_add(usize::try_from(self.size).ok()?)?;
        // The whole slot must stay below the frame pointer and inside the frame.
        (end <= fp).then_some(start)
    }
}

/// One function described by the stopped-frame table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameFunction {
    /// Linker-visible symbol name.
    pub(crate) symbol: String,
    /// Machine-code size in bytes from the function entry.
    pub(crate) code_size: u32,
    /// Size of the frame the function allocates below the frame pointer.
    pub(crate) frame_size: u32,
    /// Typed locals, sorted by normalized name.
    pub(crate) locals: Vec<FrameLocal>,
}

/// A validated stopped-frame table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrameTable {
    /// Per-function layout data, keyed by manifest function identity.
    pub(crate) functions: BTreeMap<GotId, FrameFunction>,
}

/// The result of looking for a stopped-frame sidecar beside a module.
pub(crate) enum FramesSource {
    /// No sidecar exists; release and hook-free builds omit it.
    Missing,
    /// The sidecar exists but cannot be read within its limits.
    Unreadable(String),
    /// The sidecar bytes, to be validated against the manifest build identity.
    Bytes(Vec<u8>),
}

/// Reads the stopped-frame sidecar file without reporting host paths.
pub(crate) fn read(path: &Path) -> FramesSource {
    match std::fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => FramesSource::Missing,
        Err(_) => FramesSource::Unreadable("the stopped-frame sidecar is not readable".into()),
        Ok(metadata) if metadata.len() > MAX_TABLE_BYTES => FramesSource::Unreadable(format!(
            "the stopped-frame sidecar exceeds the {MAX_TABLE_BYTES}-byte limit"
        )),
        Ok(_) => match std::fs::read(path) {
            Ok(bytes) if u64::try_from(bytes.len()).unwrap_or(u64::MAX) <= MAX_TABLE_BYTES => {
                FramesSource::Bytes(bytes)
            }
            Ok(_) => FramesSource::Unreadable(format!(
                "the stopped-frame sidecar exceeds the {MAX_TABLE_BYTES}-byte limit"
            )),
            Err(_) => FramesSource::Unreadable("the stopped-frame sidecar is not readable".into()),
        },
    }
}

/// Parses and validates a format-1 stopped-frame table.
///
/// # Errors
///
/// Returns a stable, path-free message naming the first invalid field. A
/// caller turns this into `STC-E-DEBUG-017` availability, never a load error.
pub(crate) fn parse(text: &str, build_id: GotId, target: &Target) -> Result<FrameTable, String> {
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_TABLE_BYTES {
        return Err(format!(
            "the stopped-frame sidecar exceeds the {MAX_TABLE_BYTES}-byte limit"
        ));
    }
    let root = json::parse(text).map_err(|_| "the stopped-frame table is not valid JSON")?;
    let Some(object) = root.as_object() else {
        return Err("the stopped-frame table is not a JSON object".into());
    };
    if object.len() != TABLE_KEYS.len() || TABLE_KEYS.iter().any(|key| !object.contains_key(*key)) {
        return Err("the stopped-frame table does not have the format-1 keys".into());
    }
    if object.get("format").and_then(Json::as_number) != Some(1) {
        return Err("the stopped-frame table format is not 1".into());
    }
    let table_build = object
        .get("build_id")
        .and_then(Json::as_str)
        .and_then(|value| GotId::parse(value).ok());
    if table_build != Some(build_id) {
        return Err(
            "the stopped-frame table build_id does not match the manifest build identity".into(),
        );
    }
    let pointer_width = object
        .get("pointer_width")
        .and_then(Json::as_number)
        .and_then(|value| u32::try_from(value).ok());
    if pointer_width != Some(u32::from(target.pointer_width)) || pointer_width != Some(8) {
        return Err("the stopped-frame table pointer width is not supported".into());
    }
    let table_target = object.get("target").and_then(Json::as_str);
    if table_target != Some(target.triple.as_str()) {
        return Err(
            "the stopped-frame table target does not match the active platform target".into(),
        );
    }
    let Some(functions) = object.get("functions").and_then(Json::as_array) else {
        return Err("the stopped-frame table functions field is not an array".into());
    };
    if functions.is_empty() {
        return Err("the stopped-frame table has no functions".into());
    }
    let mut table = FrameTable {
        functions: BTreeMap::new(),
    };
    let mut locals = 0usize;
    let mut previous: Option<GotId> = None;
    for function in functions {
        let Some(object) = function.as_object() else {
            return Err("a stopped-frame function is not a JSON object".into());
        };
        if object.len() != FUNCTION_KEYS.len()
            || FUNCTION_KEYS.iter().any(|key| !object.contains_key(*key))
        {
            return Err("a stopped-frame function does not have the format-1 keys".into());
        }
        let identity = object
            .get("function")
            .and_then(Json::as_str)
            .and_then(|value| GotId::parse(value).ok());
        let Some(identity) = identity else {
            return Err("a stopped-frame function identity is not a GotId".into());
        };
        if previous.is_some_and(|previous| previous >= identity) {
            return Err("stopped-frame functions are not sorted and unique".into());
        }
        previous = Some(identity);
        let symbol = object
            .get("symbol")
            .and_then(Json::as_str)
            .filter(|symbol| valid_symbol(symbol));
        let Some(symbol) = symbol else {
            return Err("a stopped-frame function symbol is not printable".into());
        };
        if object.get("low_pc").and_then(Json::as_number) != Some(0) {
            return Err("a stopped-frame function low_pc is not normalized to zero".into());
        }
        let code_size = object
            .get("high_pc")
            .and_then(Json::as_number)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|size| (1..=MAX_CODE_SIZE).contains(size));
        let Some(code_size) = code_size else {
            return Err("a stopped-frame function code size is invalid".into());
        };
        let frame_size = object
            .get("frame_size")
            .and_then(Json::as_number)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|size| (16..=MAX_FRAME_SIZE).contains(size));
        let Some(frame_size) = frame_size else {
            return Err("a stopped-frame function frame size is invalid".into());
        };
        if object.get("cfa").and_then(Json::as_str) != Some("fp+16") {
            return Err("a stopped-frame function CFA rule is not supported".into());
        }
        if object.get("saved_fp").and_then(Json::as_number) != Some(-16)
            || object.get("return_address").and_then(Json::as_number) != Some(-8)
        {
            return Err("a stopped-frame function frame offsets are not supported".into());
        }
        let Some(entries) = object.get("locals").and_then(Json::as_array) else {
            return Err("a stopped-frame function locals field is not an array".into());
        };
        locals = locals
            .checked_add(entries.len())
            .filter(|total| *total <= MAX_LOCALS)
            .ok_or_else(|| {
                format!("the stopped-frame table exceeds the {MAX_LOCALS}-local limit")
            })?;
        let mut function_locals = Vec::with_capacity(entries.len());
        let mut previous_name: Option<&str> = None;
        for entry in entries {
            let Some(entry) = entry.as_object() else {
                return Err("a stopped-frame local is not a JSON object".into());
            };
            if entry.len() != LOCAL_KEYS.len()
                || LOCAL_KEYS.iter().any(|key| !entry.contains_key(*key))
            {
                return Err("a stopped-frame local does not have the format-1 keys".into());
            }
            let name = entry
                .get("name")
                .and_then(Json::as_str)
                .filter(|name| valid_local_name(name));
            let Some(name) = name else {
                return Err("a stopped-frame local name is not normalized".into());
            };
            if previous_name.is_some_and(|previous| previous >= name) {
                return Err("stopped-frame locals are not sorted and unique".into());
            }
            previous_name = Some(name);
            let type_key = entry
                .get("type")
                .and_then(Json::as_str)
                .filter(|text| valid_type_key(text));
            let Some(type_key) = type_key else {
                return Err("a stopped-frame local type key is invalid".into());
            };
            let size = entry
                .get("size")
                .and_then(Json::as_number)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|size| (1..=frame_size).contains(size));
            let Some(size) = size else {
                return Err("a stopped-frame local size is invalid".into());
            };
            let offset = entry
                .get("location")
                .and_then(Json::as_str)
                .and_then(parse_fp_offset)
                .filter(|offset| *offset >= size && *offset <= frame_size);
            let Some(offset) = offset else {
                return Err("a stopped-frame local location is not a supported fp offset".into());
            };
            let Some(live) = entry.get("live").and_then(Json::as_object) else {
                return Err("a stopped-frame local live range is not an object".into());
            };
            if live.len() != LIVE_KEYS.len() || LIVE_KEYS.iter().any(|key| !live.contains_key(*key))
            {
                return Err(
                    "a stopped-frame local live range does not have the format-1 keys".into(),
                );
            }
            let first = live
                .get("first")
                .and_then(Json::as_number)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value >= 1);
            let last = live
                .get("last")
                .and_then(Json::as_number)
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value <= u32::try_from(MAX_LOCALS).unwrap_or(u32::MAX));
            let (Some(first), Some(last)) = (first, last) else {
                return Err("a stopped-frame local live range is invalid".into());
            };
            if first > last {
                return Err("a stopped-frame local live range is empty".into());
            }
            function_locals.push(FrameLocal {
                name: name.to_owned(),
                type_key: type_key.to_owned(),
                size,
                offset,
                live_first: first,
                live_last: last,
            });
        }
        if table
            .functions
            .insert(
                identity,
                FrameFunction {
                    symbol: symbol.to_owned(),
                    code_size,
                    frame_size,
                    locals: function_locals,
                },
            )
            .is_some()
        {
            return Err("stopped-frame functions are not sorted and unique".into());
        }
    }
    Ok(table)
}

fn valid_symbol(symbol: &str) -> bool {
    !symbol.is_empty()
        && symbol.len() <= MAX_SYMBOL_LEN
        && symbol
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$'))
}

fn valid_local_name(name: &str) -> bool {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return false;
    }
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == b'_') {
        return false;
    }
    name == name.to_ascii_uppercase()
        && name.bytes().all(|byte| {
            byte.is_ascii_uppercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'.')
        })
}

fn valid_type_key(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_TYPE_LEN
        && !text.chars().any(char::is_control)
        && TypeKey::parse(text).is_ok()
}

fn parse_fp_offset(location: &str) -> Option<u32> {
    let digits = location.strip_prefix("fp-")?;
    if digits.is_empty()
        || digits.len() > 10
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
        || digits.starts_with('0')
    {
        return None;
    }
    digits.parse::<u32>().ok().filter(|offset| *offset > 0)
}

/// Returns `true` for a relative source path that is safe to report.
pub(crate) fn valid_source_file(file: &str) -> bool {
    if file.is_empty()
        || file.len() > MAX_FILE_LEN
        || file.starts_with('/')
        || file.starts_with('\\')
    {
        return false;
    }
    let bytes = file.as_bytes();
    if bytes.get(1) == Some(&b':') && bytes[0].is_ascii_alphabetic() {
        return false;
    }
    if file.contains('\0') || file.chars().any(char::is_control) {
        return false;
    }
    !file
        .split(['/', '\\'])
        .any(|part| part.is_empty() || part == "." || part == "..")
}

#[cfg(test)]
mod tests {
    use super::*;
    use stc_got::Endianness;

    const BUILD: &str = "0x0416f02bb4ec6de022705a07a0869e4f";
    const FUNCTION: &str = "0xd3e548f92a30d8885af1fb0198585f79";

    fn target() -> Target {
        Target::new("x86_64-pc-windows-msvc", Endianness::Little, 8)
    }

    fn build() -> GotId {
        GotId::parse(BUILD).expect("build id")
    }

    fn table(locals: &str) -> String {
        format!(
            r#"{{
                "build_id": "{BUILD}",
                "format": 1,
                "functions": [
                    {{
                        "cfa": "fp+16",
                        "frame_size": 72,
                        "function": "{FUNCTION}",
                        "high_pc": 127,
                        "locals": [{locals}],
                        "low_pc": 0,
                        "return_address": -8,
                        "saved_fp": -16,
                        "symbol": "stc_fn_APP_n_ADD_9731bc2064eeb366"
                    }}
                ],
                "pointer_width": 8,
                "target": "x86_64-pc-windows-msvc"
            }}"#
        )
    }

    fn local(name: &str, location: &str) -> String {
        format!(
            r#"{{"live": {{"first": 1, "last": 2}}, "location": "{location}", "name": "{name}", "size": 4, "type": "DINT"}}"#
        )
    }

    #[test]
    fn parses_a_sorted_table() {
        let text = table(&format!("{},{}", local("A", "fp-4"), local("B", "fp-8")));
        let parsed = parse(&text, build(), &target()).expect("valid table");
        let function = &parsed.functions[&GotId::parse(FUNCTION).unwrap()];
        assert_eq!(function.code_size, 127);
        assert_eq!(function.frame_size, 72);
        assert_eq!(function.locals.len(), 2);
        assert_eq!(function.locals[0].name, "A");
        assert_eq!(function.locals[1].offset, 8);
        assert_eq!(function.locals[1].live_last, 2);
    }

    #[test]
    fn rejects_unpaired_build_identity() {
        let text = table(&local("A", "fp-4"));
        let other = GotId::parse("0x11111111111111111111111111111111").unwrap();
        assert!(parse(&text, other, &target()).is_err());
    }

    #[test]
    fn rejects_a_foreign_target() {
        let text = table(&local("A", "fp-4"));
        let other = Target::new("x86_64-unknown-linux-gnu", Endianness::Little, 8);
        assert!(parse(&text, build(), &other).is_err());
    }

    #[test]
    fn rejects_unsorted_or_duplicate_locals() {
        let text = table(&format!("{},{}", local("B", "fp-4"), local("A", "fp-8")));
        assert!(parse(&text, build(), &target()).is_err());
        let text = table(&format!("{},{}", local("A", "fp-4"), local("A", "fp-8")));
        assert!(parse(&text, build(), &target()).is_err());
    }

    #[test]
    fn rejects_location_outside_the_frame() {
        let text = table(&local("A", "fp-73"));
        assert!(parse(&text, build(), &target()).is_err());
        let text = table(&local("A", "fp-2"));
        assert!(parse(&text, build(), &target()).is_err());
        let text = table(&local("A", "fp+4"));
        assert!(parse(&text, build(), &target()).is_err());
    }

    #[test]
    fn accepts_absent_sidecar() {
        let dir = std::env::temp_dir().join(format!(
            "stc-frames-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |value| value.as_nanos())
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source = read(&dir.join("app.stc-frames.json"));
        assert!(matches!(source, FramesSource::Missing));
        std::fs::write(dir.join("bad.stc-frames.json"), b"{").unwrap();
        let source = read(&dir.join("bad.stc-frames.json"));
        assert!(matches!(source, FramesSource::Bytes(_)));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
