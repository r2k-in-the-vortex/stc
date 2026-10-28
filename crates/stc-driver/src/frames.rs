//! Build-time stopped-frame sidecar extraction (D-181, Debug and Monitoring
//! §16.9).
//!
//! The front end supplies function identity, canonical type keys, and local
//! sizes ([`stc_codegen::FrameFunctionHint`]); this module merges them with the
//! static frame locations the backend wrote to DWARF or CodeView. The result is
//! the deterministic format-1 `<name>.stc-frames.json` table. Extraction never
//! guesses: an unreadable tool, an unsupported target, or a function with no
//! usable frame description simply omits that data (or the whole table), and
//! the runtime reports stack/frame reads as unavailable.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use stc_codegen::{FrameFunctionHint, Json};

/// Frame layout shared by x86-64 and AArch64 debug builds (D-202): the
/// prologue saves the frame pointer and return address below it, so the
/// canonical frame address is `fp + 16`, the saved frame pointer sits at
/// `CFA - 16`, and the return address at `CFA - 8`.
const CFA: &str = "fp+16";
const SAVED_FP: i128 = -16;
const RETURN_ADDRESS: i128 = -8;
const MIN_FRAME_SIZE: u64 = 16;
const LOCAL_LIMIT: usize = 1_000_000;
const TABLE_LIMIT: usize = 64 * 1024 * 1024;

/// Inputs for one frame-table extraction.
pub(crate) struct ExtractRequest<'a> {
    /// The compiled relocatable object.
    pub object: &'a Path,
    /// The clang executable used for the build (its sibling holds
    /// `llvm-readobj`).
    pub clang: &'a str,
    /// The resolved module target triple.
    pub target_triple: &'a str,
    /// The manifest build identity, `0x`-prefixed.
    pub build_id: &'a str,
    /// Target pointer width in bytes.
    pub pointer_width: u8,
    /// Front-end hints, sorted by function identity.
    pub hints: &'a [FrameFunctionHint],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Format {
    CodeView,
    Dwarf,
}

/// Extracts the sidecar text, or returns `None` when the target has no v1
/// frame rules, the backend tool is unavailable, or it yields no usable
/// function.
pub(crate) fn extract(request: &ExtractRequest<'_>) -> Option<String> {
    if request.pointer_width != 8
        || !(request.target_triple.starts_with("x86_64-")
            || request.target_triple.starts_with("aarch64-"))
    {
        return None;
    }
    let format = if request.target_triple.contains("windows") {
        if !request.target_triple.contains("msvc") {
            // v1 covers Windows MSVC (CodeView) and Linux (DWARF) only.
            return None;
        }
        Format::CodeView
    } else if request.target_triple.contains("linux") {
        Format::Dwarf
    } else {
        return None;
    };
    let tool = tool_for(request.clang, format)?;
    let text = run_tool(&tool, format, request.object)?;
    let backend = match format {
        Format::CodeView => parse_codeview(&text),
        Format::Dwarf => parse_dwarf(&text),
    };
    render(request, &backend)
}

fn tool_for(clang: &str, format: Format) -> Option<PathBuf> {
    match format {
        Format::CodeView => {
            let directory = Path::new(clang).parent()?;
            for name in ["llvm-readobj.exe", "llvm-readobj"] {
                let candidate = directory.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
            // Fall back to PATH; a missing tool makes the spawn fail, which
            // omits the table rather than failing the build.
            Some(PathBuf::from("llvm-readobj"))
        }
        Format::Dwarf => Some(PathBuf::from("readelf")),
    }
}

fn run_tool(tool: &Path, format: Format, object: &Path) -> Option<String> {
    let mut command = Command::new(tool);
    match format {
        Format::CodeView => {
            command.arg("--codeview").arg(object);
        }
        Format::Dwarf => {
            command.args(["--debug-dump=info", "--wide"]).arg(object);
        }
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One backend-described function.
#[derive(Default)]
struct BackendFunction {
    symbol: String,
    code_size: u64,
    frame_size: Option<u64>,
    frame_pointer: bool,
    locals: BTreeMap<String, i64>,
}

fn render(
    request: &ExtractRequest<'_>,
    backend: &BTreeMap<String, BackendFunction>,
) -> Option<String> {
    let mut functions = Vec::new();
    let mut local_total = 0usize;
    for hint in request.hints {
        let Some(function) = backend.get(&hint.symbol) else {
            continue;
        };
        if !function.frame_pointer || function.code_size == 0 {
            continue;
        }
        let mut locals = Vec::new();
        let mut extent = 0u64;
        for local in &hint.locals {
            // A function without checkpoints has no liveness range, so its
            // locals cannot be described honestly; the function entry remains
            // useful for caller-frame identity.
            if local.first == 0 || local.last == 0 {
                continue;
            }
            let Some(offset) = function.locals.get(&local.name.to_ascii_uppercase()) else {
                continue;
            };
            if *offset >= 0 || local.size == 0 {
                continue;
            }
            let width = u64::from(local.size);
            let end = offset.unsigned_abs().checked_add(width)?;
            extent = extent.max(end);
            locals.push((
                local.name.clone(),
                Json::object([
                    (
                        "live".into(),
                        Json::object([
                            ("first".into(), Json::Number(i128::from(local.first))),
                            ("last".into(), Json::Number(i128::from(local.last))),
                        ]),
                    ),
                    ("location".into(), Json::String(format!("fp{offset}"))),
                    ("name".into(), Json::String(local.name.clone())),
                    ("size".into(), Json::Number(i128::from(local.size))),
                    ("type".into(), Json::String(local.type_key.clone())),
                ]),
            ));
        }
        locals.sort_by(|left, right| left.0.cmp(&right.0));
        local_total += locals.len();
        if local_total > LOCAL_LIMIT {
            return None;
        }
        let locals = locals
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>();
        let frame_size = function
            .frame_size
            .unwrap_or(0)
            .max(extent)
            .max(MIN_FRAME_SIZE);
        functions.push(Json::object([
            ("cfa".into(), Json::String(CFA.into())),
            ("frame_size".into(), Json::Number(i128::from(frame_size))),
            ("function".into(), Json::String(hint.function.to_hex())),
            (
                "high_pc".into(),
                Json::Number(i128::from(function.code_size)),
            ),
            ("locals".into(), Json::Array(locals)),
            ("low_pc".into(), Json::Number(0)),
            ("return_address".into(), Json::Number(RETURN_ADDRESS)),
            ("saved_fp".into(), Json::Number(SAVED_FP)),
            ("symbol".into(), Json::String(hint.symbol.clone())),
        ]));
    }
    if functions.is_empty() {
        return None;
    }
    let table = Json::object([
        ("build_id".into(), Json::String(request.build_id.to_owned())),
        ("format".into(), Json::Number(1)),
        ("functions".into(), Json::Array(functions)),
        (
            "pointer_width".into(),
            Json::Number(i128::from(request.pointer_width)),
        ),
        (
            "target".into(),
            Json::String(request.target_triple.to_owned()),
        ),
    ])
    .render();
    (table.len() <= TABLE_LIMIT).then_some(table)
}

/// Parses `llvm-readobj --codeview` text.
fn parse_codeview(text: &str) -> BTreeMap<String, BackendFunction> {
    let mut functions = BTreeMap::new();
    let mut current: Option<BackendFunction> = None;
    let mut pending_local: Option<String> = None;
    let mut pending_offset: Option<i64> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("ProcIdSym {") {
            finish_codeview(&mut functions, &mut current);
            current = Some(BackendFunction::default());
            pending_local = None;
            pending_offset = None;
        } else if line.starts_with("ProcEnd") {
            finish_codeview(&mut functions, &mut current);
            pending_local = None;
            pending_offset = None;
        } else if line == "HasFP (0x1)" {
            if let Some(function) = &mut current {
                function.frame_pointer = true;
            }
        } else if let Some(value) = line.strip_prefix("CodeSize: ") {
            if let Some(function) = &mut current {
                function.code_size = parse_hex(value).unwrap_or(0);
            }
        } else if let Some(value) = line.strip_prefix("LinkageName: ") {
            if let Some(function) = &mut current
                && function.symbol.is_empty()
            {
                function.symbol = value.trim().to_owned();
            }
        } else if line.starts_with("FrameProcSym {") {
            // The following TotalFrameBytes/LocalFramePtrReg lines apply to
            // the current procedure.
        } else if let Some(value) = line.strip_prefix("TotalFrameBytes: ") {
            if let Some(function) = &mut current {
                function.frame_size = parse_hex(value);
            }
        } else if let Some(value) = line.strip_prefix("LocalFramePtrReg: ") {
            if let Some(function) = &mut current
                && !value.trim().starts_with("RBP")
            {
                function.frame_pointer = false;
            }
        } else if line.starts_with("LocalSym {") {
            pending_local = Some(String::new());
            pending_offset = None;
        } else if let Some(value) = line.strip_prefix("VarName: ") {
            if let Some(name) = pending_local.as_mut() {
                *name = value.trim().to_ascii_uppercase();
            }
        } else if line.starts_with("DefRangeFramePointerRelSym {") {
            pending_offset = None;
        } else if let Some(value) = line.strip_prefix("Offset: ")
            && pending_local.is_some()
            && pending_offset.is_none()
        {
            pending_offset = value.trim().parse::<i64>().ok();
            if let (Some(function), Some(name), Some(offset)) =
                (current.as_mut(), pending_local.as_ref(), pending_offset)
                && !name.is_empty()
            {
                function.locals.entry(name.clone()).or_insert(offset);
                pending_local = None;
            }
        }
    }
    finish_codeview(&mut functions, &mut current);
    functions
}

fn finish_codeview(
    functions: &mut BTreeMap<String, BackendFunction>,
    current: &mut Option<BackendFunction>,
) {
    if let Some(function) = current.take()
        && !function.symbol.is_empty()
    {
        functions.insert(function.symbol.clone(), function);
    }
}

/// Parses `readelf --debug-dump=info --wide` text.
fn parse_dwarf(text: &str) -> BTreeMap<String, BackendFunction> {
    struct Subprogram {
        depth: usize,
        function: BackendFunction,
    }
    struct Local {
        depth: usize,
        name: Option<String>,
        offset: Option<i64>,
        simple: bool,
    }
    let mut functions = BTreeMap::new();
    let mut subprograms: Vec<Subprogram> = Vec::new();
    let mut local: Option<Local> = None;
    let mut last_is_subprogram = false;

    let finish_local = |subprograms: &mut Vec<Subprogram>, local: &mut Option<Local>| {
        if let Some(local) = local.take()
            && local.simple
            && let Some(name) = local.name
            && let Some(offset) = local.offset
            && let Some(subprogram) = subprograms
                .iter_mut()
                .find(|subprogram| subprogram.depth + 1 == local.depth)
        {
            subprogram
                .function
                .locals
                .entry(name.to_ascii_uppercase())
                .or_insert(offset);
        }
    };

    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some((depth, tag)) = die_tag(trimmed) {
            finish_local(&mut subprograms, &mut local);
            while subprograms
                .last()
                .is_some_and(|subprogram| subprogram.depth >= depth)
            {
                if let Some(subprogram) = subprograms.pop()
                    && !subprogram.function.symbol.is_empty()
                {
                    functions.insert(subprogram.function.symbol.clone(), subprogram.function);
                }
            }
            match tag.as_str() {
                "subprogram" => {
                    let function = BackendFunction {
                        frame_pointer: false,
                        ..BackendFunction::default()
                    };
                    subprograms.push(Subprogram { depth, function });
                    last_is_subprogram = true;
                }
                "formal_parameter" | "variable" => {
                    local = Some(Local {
                        depth,
                        name: None,
                        offset: None,
                        simple: true,
                    });
                    last_is_subprogram = false;
                }
                _ => {
                    local = None;
                    last_is_subprogram = false;
                }
            }
            continue;
        }
        let Some(attribute) = attribute_line(trimmed) else {
            continue;
        };
        if last_is_subprogram {
            let Some(subprogram) = subprograms.last_mut() else {
                continue;
            };
            let function = &mut subprogram.function;
            if attribute.starts_with("DW_AT_linkage_name") {
                if let Some(value) = attribute_value(attribute) {
                    function.symbol = value.trim().to_owned();
                }
            } else if attribute.starts_with("DW_AT_high_pc") {
                if !attribute.contains("addrx")
                    && let Some(value) = attribute_value(attribute)
                    && let Some(value) = value.trim().strip_prefix('(')
                    && let Some((_, value)) = value.split_once(')')
                {
                    function.code_size = parse_hex(value.trim()).unwrap_or(0);
                }
            } else if attribute.starts_with("DW_AT_frame_base") {
                // x86-64 names rbp (DWARF register 6); AArch64 names x29
                // (DWARF register 29). Both use the same frame offsets.
                function.frame_pointer = attribute.contains("DW_OP_reg6 (rbp)")
                    || attribute.contains("DW_OP_reg29 (x29)");
            }
        } else if let Some(local) = &mut local {
            if attribute.starts_with("DW_AT_name") {
                if let Some(value) = attribute_value(attribute) {
                    local.name = Some(value.trim().to_owned());
                }
            } else if attribute.starts_with("DW_AT_location") {
                if let Some(offset) = fbreg_offset(attribute) {
                    local.offset = Some(offset);
                } else {
                    local.simple = false;
                }
            }
        }
    }
    finish_local(&mut subprograms, &mut local);
    while let Some(subprogram) = subprograms.pop() {
        if !subprogram.function.symbol.is_empty() {
            functions.insert(subprogram.function.symbol.clone(), subprogram.function);
        }
    }
    functions
}

/// Returns the DIE depth and tag for one `<N><offset>: Abbrev Number: …` line.
fn die_tag(line: &str) -> Option<(usize, String)> {
    let rest = line.strip_prefix('<')?;
    let (depth, rest) = rest.split_once('>')?;
    let depth = depth.parse::<usize>().ok()?;
    if !rest.trim_start().starts_with('<') {
        return None;
    }
    let (_, rest) = rest.split_once("Abbrev Number:")?;
    let (_, tag) = rest.trim().split_once("DW_TAG_")?;
    let tag = tag
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .next()?;
    Some((depth, tag.to_owned()))
}

fn attribute_line(line: &str) -> Option<&str> {
    let offset = line.find("DW_AT_")?;
    Some(&line[offset..])
}

fn attribute_value(attribute: &str) -> Option<&str> {
    attribute.rsplit_once(": ").map(|(_, value)| value)
}

fn fbreg_offset(attribute: &str) -> Option<i64> {
    let (_, value) = attribute.split_once("DW_OP_fbreg:")?;
    let value = value.trim_start();
    let end = value
        .find(|character: char| !(character.is_ascii_digit() || character == '-'))
        .unwrap_or(value.len());
    value[..end].parse().ok()
}

fn parse_hex(value: &str) -> Option<u64> {
    let value = value.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    u64::from_str_radix(value, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codeview_extracts_frame_pointer_locals() {
        let text = r#"
    ProcIdSym {
      Kind: S_LPROC32_ID (0x1146)
      CodeSize: 0x5B
      Flags [ (0x1)
        HasFP (0x1)
      ]
      LinkageName: stc_fn__ROOT_n_ADD
    }
    FrameProcSym {
      Kind: S_FRAMEPROC (0x1012)
      TotalFrameBytes: 0x48
      LocalFramePtrReg: RBP (0x14E)
    }
    LocalSym {
      Kind: S_LOCAL (0x113E)
      Type: int (0x74)
      Flags [ (0x1)
        IsParameter (0x1)
      ]
      VarName: a
    }
    DefRangeFramePointerRelSym {
      Kind: S_DEFRANGE_FRAMEPOINTER_REL (0x1142)
      Offset: -4
      LocalVariableAddrRange {
        OffsetStart: .text+0xE
        ISectStart: 0x0
        Range: 0x4D
      }
    }
    LocalSym {
      Kind: S_LOCAL (0x113E)
      Flags [ (0x0)
      ]
      VarName: x
    }
    DefRangeFramePointerRelSym {
      Kind: S_DEFRANGE_FRAMEPOINTER_REL (0x1142)
      Offset: -12
    }
    ProcEnd {
      Kind: S_PROC_ID_END (0x114F)
    }
"#;
        let functions = parse_codeview(text);
        let function = functions.get("stc_fn__ROOT_n_ADD").expect("function");
        assert_eq!(function.code_size, 0x5B);
        assert_eq!(function.frame_size, Some(0x48));
        assert!(function.frame_pointer);
        assert_eq!(function.locals.get("A"), Some(&-4));
        assert_eq!(function.locals.get("X"), Some(&-12));
    }

    #[test]
    fn dwarf_extracts_frame_pointer_locals() {
        let text = r#" <1><23>: Abbrev Number: 2 (DW_TAG_subprogram)
    <24>   DW_AT_low_pc      : (addrx) (index: 0): 0
    <25>   DW_AT_high_pc     : (data4) 0x1a
    <29>   DW_AT_frame_base  : (exprloc) 1 byte block: 56 	(DW_OP_reg6 (rbp))
    <2b>   DW_AT_linkage_name: (strx1) (offset: 0x3): stc_add
 <2><33>: Abbrev Number: 3 (DW_TAG_formal_parameter)
    <34>   DW_AT_location    : (exprloc) 2 byte block: 91 7c 	(DW_OP_fbreg: -4)
    <37>   DW_AT_name        : (strx1) (offset: 0x6): a
 <2><49>: Abbrev Number: 4 (DW_TAG_variable)
    <4a>   DW_AT_location    : (exprloc) 2 byte block: 91 74 	(DW_OP_fbreg: -12)
    <4d>   DW_AT_name        : (strx1) (offset: 0x8): x
 <2><54>: Abbrev Number: 0
 <1><55>: Abbrev Number: 5 (DW_TAG_base_type)
"#;
        let functions = parse_dwarf(text);
        let function = functions.get("stc_add").expect("function");
        assert_eq!(function.code_size, 0x1A);
        assert!(function.frame_pointer);
        assert_eq!(function.locals.get("A"), Some(&-4));
        assert_eq!(function.locals.get("X"), Some(&-12));
    }

    #[test]
    fn dwarf_extracts_aarch64_frame_pointer_locals() {
        let text = r#" <1><23>: Abbrev Number: 2 (DW_TAG_subprogram)
    <25>   DW_AT_high_pc     : (data4) 0x28
    <29>   DW_AT_frame_base  : (exprloc) 1 byte block: 9d 	(DW_OP_reg29 (x29))
    <2b>   DW_AT_linkage_name: (strx1) (offset: 0x3): stc_inner
 <2><33>: Abbrev Number: 3 (DW_TAG_formal_parameter)
    <34>   DW_AT_location    : (exprloc) 2 byte block: 91 7c 	(DW_OP_fbreg: -4)
    <37>   DW_AT_name        : (strx1) (offset: 0x6): a
 <2><49>: Abbrev Number: 4 (DW_TAG_variable)
    <4a>   DW_AT_location    : (exprloc) 2 byte block: 91 74 	(DW_OP_fbreg: -12)
    <4d>   DW_AT_name        : (strx1) (offset: 0x8): x
 <2><54>: Abbrev Number: 0
"#;
        let functions = parse_dwarf(text);
        let function = functions.get("stc_inner").expect("function");
        assert_eq!(function.code_size, 0x28);
        assert!(function.frame_pointer);
        assert_eq!(function.locals.get("A"), Some(&-4));
        assert_eq!(function.locals.get("X"), Some(&-12));
    }

    #[test]
    fn non_frame_pointer_and_register_locals_are_ignored() {
        let text = r#" <1><23>: Abbrev Number: 2 (DW_TAG_subprogram)
    <25>   DW_AT_high_pc     : (data4) 0x10
    <29>   DW_AT_frame_base  : (exprloc) 1 byte block: 9c 	(DW_OP_call_frame_cfa)
    <2b>   DW_AT_linkage_name: (strx1) (offset: 0x3): stc_add
 <2><33>: Abbrev Number: 3 (DW_TAG_formal_parameter)
    <34>   DW_AT_location    : 1 byte block: 55 	(DW_OP_reg5 (rdi))
    <37>   DW_AT_name        : (strx1) (offset: 0x6): a
 <2><54>: Abbrev Number: 0
"#;
        let functions = parse_dwarf(text);
        assert!(functions["stc_add"].locals.is_empty());
        assert!(!functions["stc_add"].frame_pointer);
    }
}
