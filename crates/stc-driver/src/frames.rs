//! Build-time stopped-frame sidecar extraction (D-181, Debug and Monitoring
//! §16.9).
//!
//! The front end supplies function identity, canonical type keys, and local
//! sizes ([`stc_codegen::FrameFunctionHint`]); this module merges them with the
//! static frame locations and source line tables the backend wrote to DWARF or
//! CodeView. The result is the deterministic format-2
//! `<name>.stc-frames.json` table. Extraction never guesses: an unreadable
//! tool, an unsupported target, or a function with no usable frame
//! description simply omits that data (or the whole table), and the runtime
//! reports stack/frame reads as unavailable.

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
/// Maximum number of call-site line mappings across one frame table.
const LINE_LIMIT: usize = 1_000_000;
/// Maximum reported source-path length, matching the runtime validator.
const MAX_SOURCE_FILE_LEN: usize = 1024;
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
    MachDwarf,
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
            // v1 covers Windows MSVC (CodeView), ELF (DWARF), and Mach-O
            // (DWARF) only.
            return None;
        }
        Format::CodeView
    } else if request.target_triple.contains("linux") {
        Format::Dwarf
    } else if request.target_triple.contains("apple-darwin") {
        Format::MachDwarf
    } else {
        return None;
    };
    let tool = tool_for(request.clang, format)?;
    let text = run_tool(&tool, format, request.object)?;
    let mut backend = match format {
        Format::CodeView => parse_codeview(&text),
        Format::Dwarf => parse_dwarf(&text),
        Format::MachDwarf => parse_mach_dwarf(&text),
    };
    if format == Format::Dwarf
        && let Some(lines) = run_decoded_lines(&tool, format, request.object)
    {
        attach_dwarf_lines(&mut backend, &lines);
    } else if format == Format::MachDwarf
        && let Some(lines) = run_decoded_lines(&tool, format, request.object)
    {
        attach_decoded_lines(&mut backend, parse_mach_decoded_lines(&lines));
    }
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
        Format::MachDwarf => mach_dwarf_tool(clang),
    }
}

/// Resolves `llvm-dwarfdump` for a Mach-O target.
///
/// The command-line `PATH` wins (the macOS CI job pins Homebrew LLVM there),
/// then the active Xcode developer directory, then the LLVM tools installed
/// beside the configured clang. Returning `None` omits the sidecar rather than
/// failing the build; CI asserts that the reader exists so the tests cannot
/// skip silently.
fn mach_dwarf_tool(clang: &str) -> Option<PathBuf> {
    if let Some(tool) = find_on_path("llvm-dwarfdump") {
        return Some(tool);
    }
    if let Some(tool) = xcrun_find("llvm-dwarfdump") {
        return Some(tool);
    }
    let clang = resolve_program(clang)?;
    let directory = clang.parent()?;
    for name in ["llvm-dwarfdump", "llvm-dwarfdump.exe"] {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Finds a program on `PATH` without running it.
fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for candidate in [name.to_owned(), format!("{name}.exe")] {
            let candidate = directory.join(candidate);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Resolves a configured program name to a path when it is a bare name.
fn resolve_program(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        return Some(path.to_path_buf());
    }
    find_on_path(program)
}

/// Asks the active Xcode developer directory for a tool path.
fn xcrun_find(name: &str) -> Option<PathBuf> {
    let output = Command::new("xcrun").args(["--find", name]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!path.is_empty()).then(|| PathBuf::from(path))
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
        Format::MachDwarf => {
            command.arg("--debug-info").arg(object);
        }
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reads the DWARF line program as decoded address/file/line rows.
fn run_decoded_lines(tool: &Path, format: Format, object: &Path) -> Option<String> {
    let mut command = Command::new(tool);
    match format {
        Format::Dwarf => {
            command.args(["--debug-dump=decodedline", "--wide"]);
        }
        Format::MachDwarf => {
            command.arg("--debug-line");
        }
        Format::CodeView => return None,
    }
    let output = command.arg(object).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// One call-site line mapping, relative to the function entry.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BackendLine {
    offset: u64,
    file: String,
    line: u32,
    column: u32,
}

/// One backend-described function.
#[derive(Default)]
struct BackendFunction {
    symbol: String,
    code_size: u64,
    low_pc: Option<u64>,
    frame_size: Option<u64>,
    frame_pointer: bool,
    locals: BTreeMap<String, i64>,
    lines: Vec<BackendLine>,
}

fn render(
    request: &ExtractRequest<'_>,
    backend: &BTreeMap<String, BackendFunction>,
) -> Option<String> {
    let mut functions = Vec::new();
    let mut local_total = 0usize;
    let mut line_total = 0usize;
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
        let line_ranges = normalized_lines(&function.lines, function.code_size);
        line_total = line_total.checked_add(line_ranges.len())?;
        if line_total > LINE_LIMIT {
            return None;
        }
        let line_ranges = line_ranges
            .into_iter()
            .map(|line| {
                Json::object([
                    ("column".into(), Json::Number(i128::from(line.column))),
                    ("file".into(), Json::String(line.file.clone())),
                    ("line".into(), Json::Number(i128::from(line.line))),
                    ("offset".into(), Json::Number(i128::from(line.offset))),
                ])
            })
            .collect::<Vec<_>>();
        functions.push(Json::object([
            ("cfa".into(), Json::String(CFA.into())),
            ("frame_size".into(), Json::Number(i128::from(frame_size))),
            ("function".into(), Json::String(hint.function.to_hex())),
            (
                "high_pc".into(),
                Json::Number(i128::from(function.code_size)),
            ),
            ("line_ranges".into(), Json::Array(line_ranges)),
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
        ("format".into(), Json::Number(2)),
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

/// Sorts, deduplicates, and range-checks the call-site rows for one function.
///
/// Rows outside the function's code range, with an invalid source path, or
/// without a positive line are dropped: the runtime reports the source as
/// absent rather than guessing.
fn normalized_lines(lines: &[BackendLine], code_size: u64) -> Vec<&BackendLine> {
    let mut lines = lines
        .iter()
        .filter(|line| line.offset < code_size && line.line >= 1)
        .filter(|line| valid_source_file(&line.file))
        .collect::<Vec<_>>();
    lines.sort_by(|left, right| {
        left.offset
            .cmp(&right.offset)
            .then_with(|| left.file.cmp(&right.file))
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.column.cmp(&right.column))
    });
    lines.dedup_by_key(|line| line.offset);
    lines
}

/// Normalizes a backend-reported source path to the package-relative,
/// forward-slash form the runtime validates and reports.
///
/// Absolute, drive-qualified, parent-escaping, empty, and control-character
/// paths are rejected so a host path never enters the sidecar.
fn normalize_source_file(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_SOURCE_FILE_LEN || value.chars().any(char::is_control)
    {
        return None;
    }
    let value = value.replace('\\', "/");
    if value.starts_with('/') {
        return None;
    }
    let bytes = value.as_bytes();
    if bytes.get(1) == Some(&b':') && bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut parts = Vec::new();
    for part in value.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => parts.push(part),
        }
    }
    let file = parts.join("/");
    (!file.is_empty() && file.len() <= MAX_SOURCE_FILE_LEN).then_some(file)
}

/// Returns `true` for the same relative, printable source paths the runtime
/// accepts in a stopped-frame table.
fn valid_source_file(file: &str) -> bool {
    if file.is_empty()
        || file.len() > MAX_SOURCE_FILE_LEN
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

/// One `FunctionLineTable` block while it is being parsed.
#[derive(Default)]
struct CodeViewLines {
    symbol: String,
    file: Option<String>,
    offset: Option<u64>,
    line: Option<u32>,
    column: u32,
    entries: Vec<BackendLine>,
}

/// Finishes the pending line-table row, if it has a file, offset, and line.
fn flush_codeview_line(table: &mut CodeViewLines) {
    let offset = table.offset.take();
    let line = table.line.take();
    let column = std::mem::take(&mut table.column);
    if let (Some(offset), Some(line), Some(file)) = (offset, line, table.file.as_ref())
        && line >= 1
    {
        table.entries.push(BackendLine {
            offset,
            file: file.clone(),
            line,
            column,
        });
    }
}

fn finish_codeview_lines(
    lines: &mut Option<CodeViewLines>,
    tables: &mut BTreeMap<String, Vec<BackendLine>>,
) {
    let Some(mut table) = lines.take() else {
        return;
    };
    flush_codeview_line(&mut table);
    if !table.symbol.is_empty() && !table.entries.is_empty() {
        tables
            .entry(table.symbol)
            .or_default()
            .extend(table.entries);
    }
}

/// Parses `llvm-readobj --codeview` text.
fn parse_codeview(text: &str) -> BTreeMap<String, BackendFunction> {
    let mut functions = BTreeMap::new();
    let mut current: Option<BackendFunction> = None;
    let mut pending_local: Option<String> = None;
    let mut pending_offset: Option<i64> = None;
    let mut line_table: Option<CodeViewLines> = None;
    let mut line_tables: BTreeMap<String, Vec<BackendLine>> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("FunctionLineTable [") {
            finish_codeview_lines(&mut line_table, &mut line_tables);
            line_table = Some(CodeViewLines::default());
            continue;
        }
        // A new symbol block ends an open line table even if llvm-readobj
        // changes subsection order.
        if line.starts_with("ProcIdSym {")
            || line.starts_with("GlobalProcIdSym {")
            || line.starts_with("ProcEnd")
        {
            finish_codeview_lines(&mut line_table, &mut line_tables);
        }
        if let Some(table) = line_table.as_mut() {
            if let Some(value) = line.strip_prefix("LinkageName: ") {
                if table.symbol.is_empty() {
                    table.symbol = value.trim().to_owned();
                }
            } else if let Some(value) = line.strip_prefix("Filename: ") {
                flush_codeview_line(table);
                table.file = normalize_source_file(codeview_filename(value));
            } else if let Some(value) = line.strip_prefix('+') {
                flush_codeview_line(table);
                table.offset = value.split_whitespace().next().and_then(parse_hex);
            } else if let Some(value) = line.strip_prefix("LineNumberStart: ") {
                table.line = value.trim().parse::<u32>().ok().filter(|line| *line >= 1);
            } else if let Some(value) = line.strip_prefix("ColStart: ") {
                table.column = value.trim().parse::<u32>().unwrap_or(0);
            }
        } else if line.starts_with("ProcIdSym {") || line.starts_with("GlobalProcIdSym {") {
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
    finish_codeview_lines(&mut line_table, &mut line_tables);
    for (symbol, lines) in line_tables {
        if let Some(function) = functions.get_mut(&symbol) {
            function.lines.extend(lines);
        }
    }
    functions
}

/// Strips the trailing ` (0xNN)` file identifier llvm-readobj prints after a
/// CodeView file name.
fn codeview_filename(value: &str) -> &str {
    let value = value.trim();
    if let Some((path, suffix)) = value.rsplit_once(" (")
        && suffix.starts_with("0x")
    {
        return path.trim_end();
    }
    value
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
            } else if attribute.starts_with("DW_AT_low_pc") {
                if let Some(value) = attribute_value(attribute) {
                    function.low_pc = parse_hex(value);
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

/// One decoded DWARF line-program row.
struct DecodedLine {
    address: u64,
    file: String,
    line: u32,
    column: u32,
}

/// Parses `readelf --debug-dump=decodedline --wide` text.
///
/// Binutils prints one row per matrix entry with the file name, line number,
/// and address before the optional `View` and `Stmt` columns. Parsing from the
/// right keeps long file names (which overflow the fixed file-name column)
/// intact. Binutils does not print the column, so it is always zero.
fn parse_decoded_lines(text: &str) -> Vec<DecodedLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let Some((file, line, address)) = decoded_line_row(raw) else {
            continue;
        };
        let Some(file) = normalize_source_file(file) else {
            continue;
        };
        lines.push(DecodedLine {
            address,
            file,
            line,
            column: 0,
        });
    }
    lines
}

fn decoded_line_row(text: &str) -> Option<(&str, u32, u64)> {
    let tokens = line_tokens(text);
    let mut end = tokens.len();
    if end > 0 && tokens[end - 1].1 == "x" {
        end -= 1;
    }
    for index in (1..end).rev() {
        let Some(address) = parse_line_address(tokens[index].1) else {
            continue;
        };
        let Some(line) = tokens[index - 1]
            .1
            .parse::<u32>()
            .ok()
            .filter(|line| *line >= 1)
        else {
            continue;
        };
        let file = text[..tokens[index - 1].0].trim_end();
        if file.is_empty() {
            continue;
        }
        return Some((file, line, address));
    }
    None
}

/// Splits a line into whitespace-separated tokens with their byte offsets.
fn line_tokens(text: &str) -> Vec<(usize, &str)> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            if let Some(start) = start.take() {
                tokens.push((start, &text[start..index]));
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(start) = start {
        tokens.push((start, &text[start..]));
    }
    tokens
}

fn parse_line_address(token: &str) -> Option<u64> {
    let token = token
        .strip_prefix("0x")
        .or_else(|| token.strip_prefix("0X"))
        .unwrap_or(token);
    if token.is_empty() || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u64::from_str_radix(token, 16).ok()
}

/// Parses `llvm-dwarfdump --debug-info` text for a Mach-O object.
///
/// LLVM prints one DIE line per entry (`0x<offset>: <indent>DW_TAG_…`) and one
/// attribute line per attribute (twelve columns of offset padding, then the
/// DIE indent plus one level, `DW_AT_…`, a tab, and a parenthesized value).
/// Unlike binutils it resolves `DW_AT_high_pc` to the absolute end address, so
/// the code size is the difference from `DW_AT_low_pc`.
fn parse_mach_dwarf(text: &str) -> BTreeMap<String, BackendFunction> {
    struct Subprogram {
        depth: usize,
        function: BackendFunction,
        end_pc: Option<u64>,
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
    let finish_subprogram =
        |subprogram: Subprogram, functions: &mut BTreeMap<String, BackendFunction>| {
            let Subprogram {
                mut function,
                end_pc,
                ..
            } = subprogram;
            if function.code_size == 0
                && let (Some(low_pc), Some(end_pc)) = (function.low_pc, end_pc)
                && end_pc > low_pc
            {
                function.code_size = end_pc - low_pc;
            }
            if !function.symbol.is_empty() {
                functions.insert(function.symbol.clone(), function);
            }
        };

    for line in text.lines() {
        if let Some((depth, tag)) = mach_die_tag(line) {
            finish_local(&mut subprograms, &mut local);
            while subprograms
                .last()
                .is_some_and(|subprogram| subprogram.depth >= depth)
            {
                if let Some(subprogram) = subprograms.pop() {
                    finish_subprogram(subprogram, &mut functions);
                }
            }
            match tag.as_str() {
                "subprogram" => {
                    subprograms.push(Subprogram {
                        depth,
                        function: BackendFunction::default(),
                        end_pc: None,
                    });
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
        let Some(attribute) = mach_attribute_line(line) else {
            continue;
        };
        if last_is_subprogram {
            let Some(subprogram) = subprograms.last_mut() else {
                continue;
            };
            if attribute.starts_with("DW_AT_linkage_name") {
                if let Some(value) = mach_attribute_value(attribute)
                    && let Some(symbol) = mach_string(value)
                {
                    // Mach-O object symbols carry one leading underscore that
                    // front-end hints and the sidecar do not.
                    subprogram.function.symbol = symbol
                        .strip_prefix('_')
                        .unwrap_or(symbol.as_str())
                        .to_owned();
                }
            } else if attribute.starts_with("DW_AT_low_pc") {
                if let Some(value) = mach_attribute_value(attribute) {
                    subprogram.function.low_pc = mach_address(value);
                }
            } else if attribute.starts_with("DW_AT_high_pc") {
                if let Some(value) = mach_attribute_value(attribute) {
                    subprogram.end_pc = mach_address(value);
                }
            } else if attribute.starts_with("DW_AT_frame_base") {
                if let Some(value) = mach_attribute_value(attribute) {
                    subprogram.function.frame_pointer = mach_frame_pointer(value);
                }
            }
        } else if let Some(local) = &mut local {
            if attribute.starts_with("DW_AT_name") {
                if let Some(value) = mach_attribute_value(attribute) {
                    local.name = mach_string(value);
                }
            } else if attribute.starts_with("DW_AT_location") {
                let offset = mach_attribute_value(attribute).and_then(mach_fbreg_offset);
                if let Some(offset) = offset {
                    local.offset = Some(offset);
                } else {
                    local.simple = false;
                }
            }
        }
    }
    finish_local(&mut subprograms, &mut local);
    while let Some(subprogram) = subprograms.pop() {
        finish_subprogram(subprogram, &mut functions);
    }
    functions
}

/// Returns the DIE depth and tag for one LLVM `0x<offset>: <indent>DW_TAG_…`
/// line. Unit headers (`…: Compile Unit: …`) and child terminators do not
/// match.
fn mach_die_tag(line: &str) -> Option<(usize, String)> {
    let rest = line.strip_prefix("0x")?;
    let (offset, rest) = rest.split_once(": ")?;
    if offset.is_empty() || !offset.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    // LLVM indents each DIE level by two spaces after the fixed-width offset.
    let spaces = rest.len() - rest.trim_start().len();
    let tag = rest.trim_start().strip_prefix("DW_TAG_")?;
    let tag = tag
        .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
        .next()?;
    (!tag.is_empty()).then(|| (spaces / 2, tag.to_owned()))
}

/// Returns one LLVM attribute line (`DW_AT_…`) without its indentation.
fn mach_attribute_line(line: &str) -> Option<&str> {
    let attribute = line.trim_start();
    (line.len() - attribute.len() >= 12 && attribute.starts_with("DW_AT_")).then_some(attribute)
}

/// Returns the parenthesized value of an LLVM attribute line.
fn mach_attribute_value(attribute: &str) -> Option<&str> {
    let (_, value) = attribute.split_once("\t(")?;
    value.strip_suffix(')')
}

/// Reads an LLVM address value.
///
/// `DW_FORM_addr` prints a bare address. `DW_FORM_addrx` normally resolves to
/// a bare address too; the `indexed (…) address = ` prefix only appears in
/// verbose output or when the address table cannot be resolved, so accept it
/// defensively.
fn mach_address(value: &str) -> Option<u64> {
    let value = value.trim();
    let value = value
        .rsplit_once("address = ")
        .map_or(value, |(_, address)| address.trim());
    parse_hex(value)
}

/// Decodes a quoted, `raw_ostream::write_escaped`-escaped LLVM string value.
fn mach_string(value: &str) -> Option<String> {
    let value = value.trim();
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match characters.next()? {
            '\\' => decoded.push('\\'),
            '"' => decoded.push('"'),
            'n' => decoded.push('\n'),
            't' => decoded.push('\t'),
            digit @ '0'..='7' => {
                // LLVM always emits a full three-digit octal escape.
                let mut code = digit.to_digit(8)?;
                for _ in 0..2 {
                    code = code * 8 + characters.next()?.to_digit(8)?;
                }
                decoded.push(char::from(u8::try_from(code).ok()?));
            }
            other => decoded.push(other),
        }
    }
    Some(decoded)
}

/// Returns `true` when a frame-base expression names the frame-pointer
/// register: `DW_OP_reg6 RBP` on x86-64, `DW_OP_reg29 X29` on AArch64.
fn mach_frame_pointer(value: &str) -> bool {
    let (operation, name) = match value.trim().split_once(char::is_whitespace) {
        Some((operation, name)) => (operation, name.trim()),
        None => (value.trim(), ""),
    };
    match operation {
        "DW_OP_reg6" => name.is_empty() || name.eq_ignore_ascii_case("rbp"),
        "DW_OP_reg29" => name.is_empty() || name.eq_ignore_ascii_case("x29"),
        _ => false,
    }
}

/// Reads the signed `DW_OP_fbreg` operand LLVM prints after the opcode.
fn mach_fbreg_offset(value: &str) -> Option<i64> {
    let rest = value.trim().strip_prefix("DW_OP_fbreg")?;
    let rest = rest.trim_start();
    let end = rest
        .find(|character: char| {
            !(character.is_ascii_digit() || character == '-' || character == '+')
        })
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Parses `llvm-dwarfdump --debug-line` text for a Mach-O object.
///
/// Each `debug_line[0x…]` table starts with a prologue that lists its
/// directories and file-name table, followed by fixed-width address rows.
/// Rows name their file by the printed index directly; `end_sequence` rows
/// mark the end of a sequence rather than a call site.
fn parse_mach_decoded_lines(text: &str) -> Vec<DecodedLine> {
    let mut lines = Vec::new();
    let mut directories: BTreeMap<u32, String> = BTreeMap::new();
    let mut files: BTreeMap<u32, (String, Option<u32>)> = BTreeMap::new();
    let mut pending: Option<(u32, String, Option<u32>)> = None;
    let mut in_table = false;

    let finish_file = |pending: &mut Option<(u32, String, Option<u32>)>,
                       files: &mut BTreeMap<u32, (String, Option<u32>)>| {
        if let Some((index, name, dir_index)) = pending.take()
            && !name.is_empty()
        {
            files.insert(index, (name, dir_index));
        }
    };

    for raw in text.lines() {
        let line = raw.trim_end();
        if line.starts_with("debug_line[") {
            finish_file(&mut pending, &mut files);
            directories.clear();
            files.clear();
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        let trimmed = line.trim_start();
        if let Some(value) = trimmed.strip_prefix("include_directories[") {
            if let Some((index, value)) = value.split_once("] = ")
                && let Ok(index) = index.trim().parse::<u32>()
                && let Some(directory) = mach_string(value)
            {
                directories.insert(index, directory);
            }
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("file_names[") {
            finish_file(&mut pending, &mut files);
            if let Some((index, _)) = value.split_once("]:")
                && let Ok(index) = index.trim().parse::<u32>()
            {
                pending = Some((index, String::new(), None));
            }
            continue;
        }
        if let Some(file) = pending.as_mut() {
            if let Some(value) = trimmed.strip_prefix("name: ") {
                if let Some(name) = mach_string(value) {
                    file.1 = name;
                }
                continue;
            }
            if let Some(value) = trimmed.strip_prefix("dir_index: ") {
                file.2 = value.trim().parse::<u32>().ok();
                continue;
            }
        }
        // Address rows start with the fixed-width address column.
        if !line.starts_with("0x") {
            continue;
        }
        // The rows follow the prologue; commit the last file entry before the
        // first row so a table with a single entry can resolve its rows.
        finish_file(&mut pending, &mut files);
        let tokens = line_tokens(line);
        if tokens.len() < 7 {
            continue;
        }
        let Some(address) = parse_line_address(tokens[0].1) else {
            continue;
        };
        let Ok(line_number) = tokens[1].1.parse::<u32>() else {
            continue;
        };
        let Ok(column) = tokens[2].1.parse::<u32>() else {
            continue;
        };
        let Ok(file_index) = tokens[3].1.parse::<u32>() else {
            continue;
        };
        if line_number < 1
            || tokens
                .iter()
                .skip(7)
                .any(|(_, token)| *token == "end_sequence")
        {
            continue;
        }
        let Some((name, dir_index)) = files.get(&file_index) else {
            continue;
        };
        let Some(file) = mach_source_file(name, *dir_index, &directories) else {
            continue;
        };
        lines.push(DecodedLine {
            address,
            file,
            line: line_number,
            column,
        });
    }
    finish_file(&mut pending, &mut files);
    lines
}

/// Resolves a Mach-O line-table file entry against its directory index.
fn mach_source_file(
    name: &str,
    dir_index: Option<u32>,
    directories: &BTreeMap<u32, String>,
) -> Option<String> {
    let path = if name.starts_with('/') {
        name.to_owned()
    } else if let Some(directory) = dir_index.and_then(|index| directories.get(&index)) {
        if directory.is_empty() {
            name.to_owned()
        } else {
            format!("{directory}/{name}")
        }
    } else {
        name.to_owned()
    };
    normalize_source_file(&path)
}

/// Attributes decoded line rows to functions by their relocated code range,
/// converting each address to a function-relative offset.
fn attach_dwarf_lines(backend: &mut BTreeMap<String, BackendFunction>, text: &str) {
    attach_decoded_lines(backend, parse_decoded_lines(text));
}

/// Attributes decoded line rows to functions by their relocated code range,
/// converting each address to a function-relative offset.
fn attach_decoded_lines(backend: &mut BTreeMap<String, BackendFunction>, rows: Vec<DecodedLine>) {
    if rows.is_empty() {
        return;
    }
    let mut regions = backend
        .iter()
        .filter_map(|(symbol, function)| {
            let low_pc = function.low_pc?;
            (function.code_size > 0).then_some((symbol.clone(), low_pc, function.code_size))
        })
        .collect::<Vec<_>>();
    regions.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    // Two functions that report the same entry address cannot be told apart;
    // attributing rows to either would invent line data.
    if regions.windows(2).any(|pair| pair[0].1 == pair[1].1) {
        return;
    }
    for row in rows {
        let index = regions.partition_point(|region| region.1 <= row.address);
        let Some(index) = index.checked_sub(1) else {
            continue;
        };
        let Some((symbol, low_pc, code_size)) = regions.get(index) else {
            continue;
        };
        let Some(offset) = row.address.checked_sub(*low_pc) else {
            continue;
        };
        if offset >= *code_size {
            continue;
        }
        if let Some(function) = backend.get_mut(symbol) {
            function.lines.push(BackendLine {
                offset,
                file: row.file.clone(),
                line: row.line,
                column: row.column,
            });
        }
    }
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
    fn codeview_extracts_call_site_lines() {
        let text = r#"
    ProcIdSym {
      Kind: S_LPROC32_ID (0x1146)
      CodeSize: 0x40
      Flags [ (0x1)
        HasFP (0x1)
      ]
      LinkageName: stc_fn_APP_n_MAIN
    }
    ProcEnd {
      Kind: S_PROC_ID_END (0x114F)
    }
  FunctionLineTable [
    LinkageName: stc_fn_APP_n_MAIN
    Flags: 0x1
    CodeSize: 0x40
    FilenameSegment [
      Filename: .\src\main.st (0x0)
      +0x0 [
        LineNumberStart: 38
        LineNumberEndDelta: 0
        IsStatement: No
      ]
      +0x12 [
        LineNumberStart: 39
        LineNumberEndDelta: 0
        IsStatement: No
        ColStart: 18
        ColEnd: 0
      ]
    ]
  ]
"#;
        let functions = parse_codeview(text);
        let function = functions.get("stc_fn_APP_n_MAIN").expect("function");
        assert_eq!(function.lines.len(), 2);
        assert_eq!(function.lines[0].offset, 0);
        assert_eq!(function.lines[0].file, "src/main.st");
        assert_eq!(function.lines[0].line, 38);
        assert_eq!(function.lines[0].column, 0);
        assert_eq!(function.lines[1].offset, 0x12);
        assert_eq!(function.lines[1].line, 39);
        assert_eq!(function.lines[1].column, 18);
    }

    #[test]
    fn codeview_rejects_absolute_and_escaping_source_paths() {
        let text = r#"
    ProcIdSym {
      CodeSize: 0x20
      HasFP (0x1)
      LinkageName: stc_add
    }
    ProcEnd {
    }
    FunctionLineTable [
    LinkageName: stc_add
    CodeSize: 0x20
    FilenameSegment [
      Filename: C:\build\src\main.st (0x0)
      +0x0 [
        LineNumberStart: 1
      ]
    ]
      Filename: .\..\main.st (0x0)
      +0x4 [
        LineNumberStart: 2
      ]
    ]
"#;
        let functions = parse_codeview(text);
        assert!(functions["stc_add"].lines.is_empty());
    }

    #[test]
    fn dwarf_line_rows_map_to_function_offsets() {
        let info = r#" <1><20>: Abbrev Number: 2 (DW_TAG_subprogram)
    <24>   DW_AT_low_pc      : (addrx) (index: 0x0): 0x0
    <25>   DW_AT_high_pc     : (data4) 0x20
    <29>   DW_AT_frame_base  : (exprloc) 1 byte block: 56 	(DW_OP_reg6 (rbp))
    <2b>   DW_AT_linkage_name: (strx1) (offset: 0x3): stc_first
 <1><40>: Abbrev Number: 2 (DW_TAG_subprogram)
    <44>   DW_AT_low_pc      : (addrx) (index: 0x1): 0x20
    <45>   DW_AT_high_pc     : (data4) 0x40
    <49>   DW_AT_frame_base  : (exprloc) 1 byte block: 56 	(DW_OP_reg6 (rbp))
    <4b>   DW_AT_linkage_name: (strx1) (offset: 0x8): stc_second
"#;
        let lines = r#"Contents of the .debug_line section:

CU: src/main.st:
File name                            Line number    Starting address    View    Stmt
src/main.st                                     1            0x000000               x
src/main.st                                     3            0x000010               x
src/very/long/directory/name/main.st           10            0x000020               x
src/very/long/directory/name/main.st           12            0x000039   2           x
"#;
        let mut functions = parse_dwarf(info);
        attach_dwarf_lines(&mut functions, lines);
        let first = &functions["stc_first"];
        assert_eq!(
            first
                .lines
                .iter()
                .map(|line| (line.offset, line.line))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0x10, 3)]
        );
        let second = &functions["stc_second"];
        assert_eq!(
            second
                .lines
                .iter()
                .map(|line| (line.offset, line.line))
                .collect::<Vec<_>>(),
            vec![(0, 10), (0x19, 12)]
        );
        assert_eq!(second.lines[1].file, "src/very/long/directory/name/main.st");
        assert_eq!(second.lines[1].column, 0);
    }

    #[test]
    fn normalizes_backend_source_paths() {
        assert_eq!(
            normalize_source_file(r".\src\main.st").as_deref(),
            Some("src/main.st")
        );
        assert_eq!(
            normalize_source_file("src\\nested/./main.st").as_deref(),
            Some("src/nested/main.st")
        );
        assert_eq!(normalize_source_file(r"C:\src\main.st"), None);
        assert_eq!(normalize_source_file("/src/main.st"), None);
        assert_eq!(normalize_source_file(r"\\server\src\main.st"), None);
        assert_eq!(normalize_source_file("../src/main.st"), None);
        assert_eq!(normalize_source_file(""), None);
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

    #[test]
    fn mach_dwarf_extracts_frame_pointer_locals() {
        let text = r#"t.o:	file format Mach-O 64-bit x86-64

.debug_info contents:
0x00000000: Compile Unit: length = 0x0000007c, format = DWARF32, version = 0x0005, unit_type = DW_UT_compile, abbr_offset = 0x0000, addr_size = 0x08 (next unit at 0x00000080)

0x0000000c: DW_TAG_compile_unit
              DW_AT_producer	("clang version 18.1.8")
              DW_AT_language	(DW_LANG_C11)
              DW_AT_name	("t.c")
              DW_AT_comp_dir	(".")
              DW_AT_low_pc	(0x0000000000000000)
              DW_AT_high_pc	(0x000000000000003b)

0x0000002d:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000000)
                DW_AT_high_pc	(0x0000000000000018)
                DW_AT_frame_base	(DW_OP_reg6 RBP)
                DW_AT_linkage_name	("stc_add")
                DW_AT_name	("add")

0x00000048:     DW_TAG_formal_parameter
                  DW_AT_location	(DW_OP_fbreg -4)
                  DW_AT_name	("a")

0x00000056:     DW_TAG_formal_parameter
                  DW_AT_location	(DW_OP_fbreg -8)
                  DW_AT_name	("b")

0x00000064:     DW_TAG_variable
                  DW_AT_location	(DW_OP_fbreg -12)
                  DW_AT_name	("sum")

0x00000072:     NULL

0x00000077:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000020)
                DW_AT_high_pc	(0x0000000000000048)
                DW_AT_frame_base	(DW_OP_reg6 RBP)
                DW_AT_linkage_name	("_stc_second")
                DW_AT_name	("second")

0x0000009a:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000050)
                DW_AT_high_pc	(0x0000000000000068)
                DW_AT_frame_base	(DW_OP_call_frame_cfa)
                DW_AT_linkage_name	("stc_leaf")
"#;
        let functions = parse_mach_dwarf(text);
        let add = functions.get("stc_add").expect("add");
        assert_eq!(add.low_pc, Some(0));
        assert_eq!(add.code_size, 0x18);
        assert!(add.frame_pointer);
        assert_eq!(add.locals.get("A"), Some(&-4));
        assert_eq!(add.locals.get("B"), Some(&-8));
        assert_eq!(add.locals.get("SUM"), Some(&-12));
        // The Mach-O symbol prefix is stripped; the sidecar keeps the
        // front-end spelling.
        let second = functions.get("stc_second").expect("second");
        assert_eq!(second.code_size, 0x28);
        assert!(second.frame_pointer);
        let leaf = functions.get("stc_leaf").expect("leaf");
        assert!(!leaf.frame_pointer);
    }

    #[test]
    fn mach_dwarf_extracts_aarch64_frame_pointer_locals() {
        let text = r#"t.o:	file format Mach-O 64-bit arm64

.debug_info contents:
0x0000000c: DW_TAG_compile_unit
              DW_AT_name	("t.c")

0x0000002d:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000000)
                DW_AT_high_pc	(0x0000000000000028)
                DW_AT_frame_base	(DW_OP_reg29 X29)
                DW_AT_linkage_name	("stc_inner")

0x00000048:     DW_TAG_formal_parameter
                  DW_AT_location	(DW_OP_fbreg -4)
                  DW_AT_name	("a")

0x00000056:     DW_TAG_variable
                  DW_AT_location	(DW_OP_fbreg -12)
                  DW_AT_name	("x")
"#;
        let functions = parse_mach_dwarf(text);
        let function = functions.get("stc_inner").expect("function");
        assert_eq!(function.code_size, 0x28);
        assert!(function.frame_pointer);
        assert_eq!(function.locals.get("A"), Some(&-4));
        assert_eq!(function.locals.get("X"), Some(&-12));
    }

    #[test]
    fn mach_dwarf_line_rows_map_to_function_offsets() {
        let info = r#"0x0000000c: DW_TAG_compile_unit
              DW_AT_name	("t.c")

0x0000002d:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000000)
                DW_AT_high_pc	(0x0000000000000020)
                DW_AT_frame_base	(DW_OP_reg6 RBP)
                DW_AT_linkage_name	("stc_first")

0x0000004a:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000020)
                DW_AT_high_pc	(0x0000000000000040)
                DW_AT_frame_base	(DW_OP_reg6 RBP)
                DW_AT_linkage_name	("stc_second")
"#;
        let lines = r#"t.o:	file format Mach-O 64-bit x86-64

.debug_line contents:
debug_line[0x00000000]
Line table prologue:
    total_length: 0x00000094
          format: DWARF32
         version: 5
    address_size: 8
 seg_select_size: 0
 prologue_length: 0x00000058
 min_inst_length: 1
max_ops_per_inst: 1
 default_is_stmt: 1
       line_base: -5
      line_range: 14
     opcode_base: 13
standard_opcode_lengths[DW_LNS_copy] = 0
include_directories[  0] = "."
file_names[  0]:
           name: "./src/main.st"
      dir_index: 0
   md5_checksum: 00000000000000000000000000000000
file_names[  1]:
           name: "/host/src/other.st"
      dir_index: 0
   md5_checksum: 00000000000000000000000000000000

Address            Line   Column File   ISA Discriminator OpIndex Flags
------------------ ------ ------ ------ --- ------------- ------- -------------
0x0000000000000000      1      0      0   0             0       0  is_stmt
0x0000000000000004      3      7      0   0             0       0  is_stmt
0x0000000000000010      9      2      1   0             0       0  is_stmt
0x0000000000000018      0      0      0   0             0       0  end_sequence
0x0000000000000020     10      0      0   0             0       0  is_stmt
0x0000000000000024     12      1      0   0             0       0  is_stmt
0x0000000000000030      0      0      0   0             0       0  end_sequence
"#;
        let mut functions = parse_mach_dwarf(info);
        attach_decoded_lines(&mut functions, parse_mach_decoded_lines(lines));
        let first = &functions["stc_first"];
        assert_eq!(
            first
                .lines
                .iter()
                .map(|line| (line.offset, line.line, line.column))
                .collect::<Vec<_>>(),
            vec![(0, 1, 0), (0x4, 3, 7)]
        );
        assert!(first.lines.iter().all(|line| line.file == "src/main.st"));
        // The row at 0x10 names a host-absolute file and is dropped by the
        // existing source-path rules; the end-sequence rows are not call
        // sites.
        let second = &functions["stc_second"];
        assert_eq!(
            second
                .lines
                .iter()
                .map(|line| (line.offset, line.line, line.column))
                .collect::<Vec<_>>(),
            vec![(0, 10, 0), (0x4, 12, 1)]
        );
        assert_eq!(second.lines[1].file, "src/main.st");
    }

    #[test]
    fn mach_dwarf_ambiguous_entry_addresses_drop_lines() {
        let info = r#"0x0000002d:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000000)
                DW_AT_high_pc	(0x0000000000000020)
                DW_AT_frame_base	(DW_OP_reg6 RBP)
                DW_AT_linkage_name	("stc_first")

0x0000004a:   DW_TAG_subprogram
                DW_AT_low_pc	(0x0000000000000000)
                DW_AT_high_pc	(0x0000000000000020)
                DW_AT_frame_base	(DW_OP_reg6 RBP)
                DW_AT_linkage_name	("stc_second")
"#;
        let lines = r#"debug_line[0x00000000]
include_directories[  0] = "."
file_names[  0]:
           name: "src/main.st"
      dir_index: 0

Address            Line   Column File   ISA Discriminator OpIndex Flags
------------------ ------ ------ ------ --- ------------- ------- -------------
0x0000000000000000      1      0      0   0             0       0  is_stmt
"#;
        let mut functions = parse_mach_dwarf(info);
        assert_eq!(functions.len(), 2);
        let rows = parse_mach_decoded_lines(lines);
        assert_eq!(
            rows.len(),
            1,
            "a table with one file entry must still resolve its rows"
        );
        assert_eq!(rows[0].file, "src/main.st");
        attach_decoded_lines(&mut functions, rows);
        assert!(
            functions.values().all(|function| function.lines.is_empty()),
            "an unresolved entry address must not attribute rows to a function"
        );
    }

    #[test]
    fn mach_address_accepts_resolved_and_indexed_forms() {
        assert_eq!(mach_address("0x0000000000000020"), Some(0x20));
        assert_eq!(
            mach_address("indexed (00000001) address = 0x0000000000000020"),
            Some(0x20)
        );
        assert_eq!(mach_address("<unresolved>"), None);
    }

    #[test]
    fn mach_strings_decode_llvm_escapes() {
        assert_eq!(mach_string(r#""a\\b""#).as_deref(), Some(r"a\b"));
        assert_eq!(mach_string(r#""a\"b""#).as_deref(), Some("a\"b"));
        assert_eq!(mach_string(r#""a\tb""#).as_deref(), Some("a\tb"));
        assert_eq!(mach_string("\"a\\141b\"").as_deref(), Some("aab"));
        assert_eq!(mach_string("not quoted"), None);
    }
}
