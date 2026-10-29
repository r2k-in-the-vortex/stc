//! IDE query tests over the committed examples.
//!
//! The tests load `examples/hello` and `examples/motor` in place with
//! `load_project_readonly`; nothing is written. Overlay tests replace unsaved
//! buffer text only.

use std::path::{Path, PathBuf};

use stc_ide::{CompletionItem, CompletionKind, DocumentSymbol, Snapshot, SymbolKind};
use stc_span::FileId;

fn manifest(example: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(example)
        .join("stc.toml")
}

fn file_id(snapshot: &Snapshot, suffix: &str) -> FileId {
    snapshot
        .files()
        .iter()
        .find(|info| info.display.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing source {suffix:?}"))
        .id
}

fn text(snapshot: &Snapshot, file: FileId) -> &str {
    snapshot.source(file).expect("project source text")
}

fn find(source: &str, needle: &str) -> usize {
    source
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"))
}

/// Byte offsets of whole-identifier, case-insensitive occurrences.
fn occurrences(source: &str, name: &str) -> Vec<usize> {
    let lowered = source.to_ascii_lowercase();
    let needle = name.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let mut offsets = Vec::new();
    for (index, _) in lowered.match_indices(&needle) {
        let before = index.checked_sub(1).map(|index| bytes[index]);
        let after = bytes.get(index + needle.len()).copied();
        if !before.is_some_and(is_ident_byte) && !after.is_some_and(is_ident_byte) {
            offsets.push(index);
        }
    }
    offsets
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn item<'a>(items: &'a [CompletionItem], label: &str) -> Option<&'a CompletionItem> {
    items
        .iter()
        .find(|item| item.label.eq_ignore_ascii_case(label))
}

fn symbol<'a>(symbols: &'a [DocumentSymbol], name: &str) -> Option<&'a DocumentSymbol> {
    symbols
        .iter()
        .find(|symbol| symbol.name.eq_ignore_ascii_case(name))
}

fn symbol_names(symbols: &[DocumentSymbol]) -> Vec<&str> {
    symbols.iter().map(|symbol| symbol.name.as_str()).collect()
}

fn assert_clean(snapshot: &Snapshot, file: FileId) {
    let diagnostics = snapshot.diagnostics(file);
    assert!(
        diagnostics.is_empty(),
        "unexpected diagnostics in {}: {diagnostics:?}",
        snapshot.file(file).expect("file").display
    );
}

#[test]
fn hello_snapshot_has_expected_files_and_no_diagnostics() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let tests = file_id(&snapshot, "hello/tests/hello_test.st");
    let info = snapshot.file(main).expect("main info");
    assert_eq!(info.display, "hello/src/main.st");
    assert_eq!(info.package, "hello");
    assert!(!info.is_test);
    assert!(snapshot.file(tests).expect("test info").is_test);
    assert_eq!(snapshot.files().len(), 2);
    assert_eq!(snapshot.file_id("hello/src/main.st"), Some(main));
    assert_eq!(snapshot.file_id_for_path(&info.path), Some(main));
    assert_eq!(
        snapshot.source_map().file_path(main),
        Some("hello/src/main.st")
    );
    assert_clean(&snapshot, main);
    assert_clean(&snapshot, tests);
    assert!(snapshot.manifest_diagnostics().is_empty());
}

#[test]
fn hello_definition_from_use_to_declaration() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let source = text(&snapshot, main);
    let use_offset = find(source, "AddTwo(sum");
    let declaration_offset = find(source, "FUNCTION AddTwo") + "FUNCTION ".len();
    let definition = snapshot.definition(main, use_offset).expect("definition");
    assert_eq!(definition.file, main);
    assert_eq!(definition.span.start as usize, declaration_offset);
    assert_eq!(
        definition.span.end as usize,
        declaration_offset + "AddTwo".len()
    );
    let expected = snapshot
        .location(main, declaration_offset)
        .expect("declaration location");
    assert_eq!(definition.line, expected.line);
    assert_eq!(definition.column, expected.column);

    // A cursor inside the declaration resolves to itself.
    let self_definition = snapshot
        .definition(main, declaration_offset)
        .expect("self definition");
    assert_eq!(self_definition.span.start, definition.span.start);

    // A cursor on an unresolved keyword has no definition.
    assert!(
        snapshot
            .definition(main, find(source, "PROGRAM Main"))
            .is_none()
    );
}

#[test]
fn hello_references_include_uses_and_optionally_the_declaration() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let tests = file_id(&snapshot, "hello/tests/hello_test.st");
    let source = text(&snapshot, main);
    let use_offset = find(source, "AddTwo(sum");

    let uses = snapshot.references(main, use_offset, false);
    let test_source = text(&snapshot, tests);
    let mut expected: Vec<(FileId, usize)> = vec![
        (main, find(source, "AddTwo := a + b")),
        (main, find(source, "AddTwo(sum")),
        (
            tests,
            find(test_source, "result := AddTwo(2, 3)") + "result := ".len(),
        ),
    ];
    expected.sort();
    let actual: Vec<(FileId, usize)> = uses
        .iter()
        .map(|location| (location.file, location.span.start as usize))
        .collect();
    assert_eq!(actual, expected);

    let with_declaration = snapshot.references(main, use_offset, true);
    assert_eq!(with_declaration.len(), uses.len() + 1);
    assert!(with_declaration.iter().any(|location| {
        location.file == main
            && location.span.start as usize == find(source, "FUNCTION AddTwo") + "FUNCTION ".len()
    }));

    // References from the declaration itself are the same set.
    let from_declaration = snapshot.references(
        main,
        find(source, "FUNCTION AddTwo") + "FUNCTION ".len(),
        false,
    );
    assert_eq!(
        from_declaration
            .iter()
            .map(|location| (location.file, location.span.start as usize))
            .collect::<Vec<_>>(),
        {
            let mut sorted = expected.clone();
            sorted.sort();
            sorted
        }
    );
}

#[test]
fn hello_references_cover_globals_and_constants() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let source = text(&snapshot, main);
    let declaration = find(source, "INCREMENT : DINT");
    let references = snapshot.references(main, declaration, true);
    let mut expected = vec![
        declaration,
        find(source, "INCREMENT, value"),
        find(source, "INCREMENT);"),
    ];
    expected.sort_unstable();
    assert_eq!(
        references
            .iter()
            .map(|location| location.span.start as usize)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(references.len(), 3, "declaration plus two uses");
}

#[test]
fn hello_hover_reports_type_kind_and_owner() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let source = text(&snapshot, main);

    let local = snapshot
        .hover(main, find(source, "sum := AddTwo"))
        .expect("hover on the `sum` field");
    assert_eq!(local.contents, "sum : DINT\nfield of program Main");

    let global = snapshot
        .hover(main, find(source, "INCREMENT : DINT"))
        .expect("hover on the global constant");
    assert_eq!(global.contents, "INCREMENT : DINT\nglobal variable");

    let function = snapshot
        .hover(main, find(source, "AddTwo := a + b"))
        .expect("hover on the function result name");
    assert_eq!(function.contents, "AddTwo : DINT\nfunction");

    let parameter = snapshot
        .hover(main, find(source, "increment : DINT := 1"))
        .expect("hover on the field `increment`");
    assert_eq!(
        parameter.contents,
        "increment : DINT\nfield of function block Counter"
    );

    // Hover only applies to identifiers.
    assert!(
        snapshot
            .hover(main, find(source, "FUNCTION AddTwo"))
            .is_none()
    );
}

#[test]
fn hello_completion_offers_keywords_names_and_scope_members() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let source = text(&snapshot, main);
    let offset = find(source, "sum := AddTwo(sum, INCREMENT)");
    let items = snapshot.completion(main, offset);

    let keyword = item(&items, "DINT").expect("a keyword");
    assert_eq!(keyword.kind, CompletionKind::Keyword);
    assert_eq!(keyword.detail, None);

    let function = item(&items, "AddTwo").expect("package function");
    assert_eq!(function.kind, CompletionKind::Function);
    assert_eq!(function.detail.as_deref(), Some("DINT"));

    let global = item(&items, "INCREMENT").expect("package global");
    assert_eq!(global.kind, CompletionKind::Variable);

    let field = item(&items, "counter").expect("program field");
    assert_eq!(field.kind, CompletionKind::Field);
    assert_eq!(field.detail.as_deref(), Some("Counter"));

    // Deduplicated by spelling.
    assert_eq!(
        items
            .iter()
            .filter(|item| item.label.eq_ignore_ascii_case("sum"))
            .count(),
        1
    );
}

#[test]
fn hello_document_symbols_form_a_hierarchy() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let symbols = snapshot.document_symbols(main);

    assert_eq!(
        symbol_names(&symbols),
        ["INCREMENT", "AddTwo", "Counter", "Main", "Hello"]
    );
    let increment = symbol(&symbols, "INCREMENT").expect("constant");
    assert_eq!(increment.kind, SymbolKind::Constant);
    assert_eq!(increment.detail.as_deref(), Some("DINT"));

    let add_two = symbol(&symbols, "AddTwo").expect("function");
    assert_eq!(add_two.kind, SymbolKind::Function);
    assert_eq!(add_two.detail.as_deref(), Some("DINT"));
    assert_eq!(symbol_names(&add_two.children), ["a", "b"]);
    assert_eq!(
        add_two.children[0].detail.as_deref(),
        Some("DINT"),
        "function parameters are variables under the function"
    );

    let main_program = symbol(&symbols, "Main").expect("program");
    assert_eq!(main_program.kind, SymbolKind::Program);
    assert_eq!(
        symbol_names(&main_program.children),
        ["counter", "sum", "blink", "lamp", "i"]
    );
    for child in &main_program.children {
        assert_eq!(child.kind, SymbolKind::Variable);
        assert!(
            matches!(
                child.detail.as_deref(),
                Some("Counter" | "DINT" | "TON" | "BOOL")
            ),
            "unexpected detail {:?}",
            child.detail
        );
    }

    let configuration = symbol(&symbols, "Hello").expect("configuration");
    assert_eq!(configuration.kind, SymbolKind::Configuration);
    let resource = symbol(&configuration.children, "Cpu").expect("resource");
    assert_eq!(resource.kind, SymbolKind::Resource);
    let task = symbol(&resource.children, "MainTask").expect("task");
    assert_eq!(task.kind, SymbolKind::Task);
    let instance = symbol(&resource.children, "hello").expect("program instance");
    assert_eq!(instance.kind, SymbolKind::ProgramInstance);
    assert_eq!(instance.detail.as_deref(), Some("Main"));

    // Selection spans point at the name token inside the declaration range.
    let selection = add_two.selection;
    assert_eq!(
        &text(&snapshot, main)[selection.start as usize..selection.end as usize],
        "AddTwo"
    );
    assert!(add_two.range.start <= selection.start && selection.end <= add_two.range.end);
}

#[test]
fn hello_formatting_is_idempotent() {
    let snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let source = text(&snapshot, main).to_owned();
    let result = snapshot.formatting(main).expect("formatting");
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(result.formatted.as_deref(), Some(source.as_str()));
    assert!(
        result.edit().is_none(),
        "an already formatted file has no edit"
    );

    // Formatting the formatted text again is stable.
    let formatted = result.formatted.expect("formatted text");
    let mut second = Snapshot::load(manifest("hello")).expect("hello loads");
    second
        .set_overlay(
            snapshot.file(main).expect("info").path.clone(),
            formatted.clone(),
        )
        .expect("overlay");
    let again = second.formatting(main).expect("formatting");
    assert_eq!(again.formatted.as_deref(), Some(formatted.as_str()));
    assert!(again.edit().is_none());
}

#[test]
fn motor_cross_package_definition_and_references() {
    let snapshot = Snapshot::load(manifest("motor")).expect("motor loads");
    let main = file_id(&snapshot, "motor/src/motor.st");
    let tests = file_id(&snapshot, "motor/tests/motor_test.st");
    let library = file_id(&snapshot, "motorlib/src/motorlib.st");
    assert_clean(&snapshot, main);
    assert_clean(&snapshot, tests);
    assert_clean(&snapshot, library);
    assert_eq!(snapshot.files().len(), 3);

    let main_source = text(&snapshot, main);
    let use_offset = find(main_source, "drive : Drive") + "drive : ".len();
    let definition = snapshot.definition(main, use_offset).expect("definition");
    assert_eq!(definition.file, library);
    let library_source = text(&snapshot, library);
    assert_eq!(
        definition.span.start as usize,
        find(library_source, "CLASS Drive") + "CLASS ".len()
    );

    let uses = snapshot.references(main, use_offset, false);
    let test_source = text(&snapshot, tests);
    let mut expected: Vec<(FileId, usize)> =
        vec![(main, find(main_source, "drive : Drive") + "drive : ".len())];
    expected.extend(
        occurrences(test_source, "motorlib.Drive")
            .into_iter()
            .map(|offset| (tests, offset + "motorlib.".len())),
    );
    expected.sort();
    let actual: Vec<(FileId, usize)> = uses
        .iter()
        .map(|location| (location.file, location.span.start as usize))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn motor_member_completion_uses_the_receiver_type() {
    let snapshot = Snapshot::load(manifest("motor")).expect("motor loads");
    let main = file_id(&snapshot, "motor/src/motor.st");
    let source = text(&snapshot, main);
    let offset = find(source, "drive.state") + "drive.".len();
    let items = snapshot.completion(main, offset);
    for expected in ["Start", "Stop", "state", "speed", "label", "ramp"] {
        assert!(
            item(&items, expected).is_some(),
            "missing member {expected}"
        );
    }
    assert_eq!(
        item(&items, "Start").expect("Start").kind,
        CompletionKind::Method
    );
    assert_eq!(
        item(&items, "state").expect("state").kind,
        CompletionKind::Field
    );
    assert_eq!(
        item(&items, "state").expect("state").detail.as_deref(),
        Some("DriveState")
    );
    assert!(
        item(&items, "DINT").is_none(),
        "member completion is not general"
    );
    assert!(item(&items, "Motor").is_none());
}

#[test]
fn motor_completion_includes_using_names() {
    let snapshot = Snapshot::load(manifest("motor")).expect("motor loads");
    let main = file_id(&snapshot, "motor/src/motor.st");
    let source = text(&snapshot, main);
    let items = snapshot.completion(main, find(source, "IF Ready THEN"));
    for expected in ["DriveState", "Describe", "Ramp", "IDrive", "motorlib"] {
        assert!(
            item(&items, expected).is_some(),
            "`USING motorlib` should expose {expected}"
        );
    }
    assert_eq!(
        item(&items, "Describe").expect("Describe").kind,
        CompletionKind::Function
    );
    assert_eq!(
        item(&items, "motorlib").expect("namespace").kind,
        CompletionKind::Namespace
    );
    // Names from packages that are not imported or depended on are absent.
    assert!(item(&items, "AddTwo").is_none());
}

#[test]
fn motor_hover_reaches_dependency_declarations() {
    let snapshot = Snapshot::load(manifest("motor")).expect("motor loads");
    let main = file_id(&snapshot, "motor/src/motor.st");
    let source = text(&snapshot, main);
    let drive = snapshot
        .hover(main, find(source, "drive.state"))
        .expect("hover on the field drive");
    assert_eq!(drive.contents, "drive : Drive\nfield of program Motor");

    let member = snapshot
        .hover(main, find(source, "drive.state") + "drive.".len())
        .expect("hover on the member state");
    assert_eq!(member.contents, "state : DriveState\nfield of class Drive");

    let start = snapshot
        .hover(main, find(source, "drive.Start") + "drive.".len())
        .expect("hover on the method Start");
    assert_eq!(start.contents, "Start : BOOL\nmethod of class Drive");

    let tests = file_id(&snapshot, "motor/tests/motor_test.st");
    let test_source = text(&snapshot, tests);
    let running = snapshot
        .hover(
            tests,
            find(test_source, "motorlib.Running") + "motorlib.".len(),
        )
        .expect("hover on the enum value");
    assert_eq!(
        running.contents,
        "Running : DriveState\nenum value of DriveState"
    );
}

#[test]
fn motor_document_symbols_cover_program_and_configuration() {
    let snapshot = Snapshot::load(manifest("motor")).expect("motor loads");
    let main = file_id(&snapshot, "motor/src/motor.st");
    let symbols = snapshot.document_symbols(main);
    assert_eq!(symbol_names(&symbols), ["Motor", "MotorDemo"]);

    let program = symbol(&symbols, "Motor").expect("program");
    assert_eq!(program.kind, SymbolKind::Program);
    let fields = symbol_names(&program.children);
    for field in [
        "Ready",
        "Enable",
        "drive",
        "target",
        "state",
        "display",
        "cycle_count",
        "total_cycles",
    ] {
        assert!(fields.contains(&field), "missing field {field}: {fields:?}");
    }
    assert_eq!(
        symbol(&program.children, "drive")
            .expect("drive field")
            .detail
            .as_deref(),
        Some("Drive")
    );

    let library = file_id(&snapshot, "motorlib/src/motorlib.st");
    let library_symbols = snapshot.document_symbols(library);
    let drive_state = symbol(&library_symbols, "DriveState").expect("enum type");
    assert_eq!(drive_state.kind, SymbolKind::Type);
    assert_eq!(
        symbol_names(&drive_state.children),
        ["Idle", "Running", "Faulted"]
    );
    let drive = symbol(&library_symbols, "Drive").expect("class");
    assert_eq!(drive.kind, SymbolKind::Class);
    let members = symbol_names(&drive.children);
    for member in ["state", "speed", "label", "ramp", "Start", "Stop"] {
        assert!(members.contains(&member), "missing member {member}");
    }
}

#[test]
fn overlay_changes_diagnostics_and_queries_without_writing() {
    let mut snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let path = snapshot.file(main).expect("info").path.clone();
    let disk = std::fs::read(&path).expect("read main.st");
    let source = text(&snapshot, main).to_owned();
    assert_clean(&snapshot, main);
    assert_eq!(
        snapshot
            .references(main, find(&source, "AddTwo(sum"), false)
            .len(),
        3
    );

    let overlay = source.replacen("sum := AddTwo(sum, INCREMENT);", "sum := MissingName;", 1);
    assert_ne!(overlay, source);
    assert_eq!(snapshot.set_overlay(&path, overlay.clone()), Some(main));
    assert!(snapshot.is_overlaid(main));
    assert_eq!(snapshot.source(main), Some(overlay.as_str()));
    assert_eq!(
        std::fs::read(&path).expect("read main.st"),
        disk,
        "overlays must not write to disk"
    );

    let diagnostics = snapshot.diagnostics(main);
    assert!(
        diagnostics.iter().any(|diagnostic| diagnostic.is_error()),
        "the unknown name is reported: {diagnostics:?}"
    );
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("MissingName")),
        "the diagnostic names the unknown identifier: {diagnostics:?}"
    );
    assert!(
        snapshot
            .hover(main, find(&overlay, "MissingName"))
            .is_none()
    );
    assert_eq!(
        snapshot
            .references(
                main,
                find(&overlay, "FUNCTION AddTwo") + "FUNCTION ".len(),
                false,
            )
            .len(),
        2,
        "the overlaid call site no longer references AddTwo"
    );

    assert_eq!(snapshot.clear_overlay(&path), Some(main));
    assert!(!snapshot.is_overlaid(main));
    assert_clean(&snapshot, main);
    assert_eq!(std::fs::read(&path).expect("read main.st"), disk);
    assert_eq!(
        snapshot
            .references(main, find(&source, "AddTwo(sum"), false)
            .len(),
        3
    );
}

#[test]
fn overlay_formatting_refuses_syntax_errors_and_is_idempotent() {
    let mut snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let path = snapshot.file(main).expect("info").path.clone();
    let source = text(&snapshot, main).to_owned();

    let unformatted = source.replacen(
        "  sum := AddTwo(sum, INCREMENT);",
        "  sum:=AddTwo(sum,INCREMENT);",
        1,
    );
    snapshot.set_overlay(&path, unformatted);
    let result = snapshot.formatting(main).expect("formatting");
    let edit = result.edit().expect("the overlay needs formatting");
    assert_eq!(edit.start, 0);
    assert_eq!(edit.end as usize, result.source.len());
    assert!(edit.new_text.contains("sum := AddTwo(sum, INCREMENT);"));

    snapshot.set_overlay(&path, edit.new_text.clone());
    let again = snapshot.formatting(main).expect("formatting");
    assert!(again.edit().is_none(), "formatting the edit is stable");

    let broken = source.replacen("END_PROGRAM", "", 1);
    snapshot.set_overlay(&path, broken);
    let refused = snapshot.formatting(main).expect("formatting");
    assert!(refused.formatted.is_none());
    assert!(refused.edit().is_none());
    assert!(
        refused
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "STC-E-FMT-001"),
        "{:?}",
        refused.diagnostics
    );
}

#[test]
fn positions_count_unicode_scalars_in_comments_and_strings() {
    let mut snapshot = Snapshot::load(manifest("hello")).expect("hello loads");
    let main = file_id(&snapshot, "hello/src/main.st");
    let path = snapshot.file(main).expect("info").path.clone();
    let source = text(&snapshot, main).to_owned();

    let line = "  (* ünïcode *) sum := AddTwo(sum, INCREMENT);";
    let overlay = source.replacen("  sum := AddTwo(sum, INCREMENT);", line, 1);
    assert_ne!(overlay, source);
    snapshot.set_overlay(&path, overlay.clone());

    let sum = find(&overlay, line) + "  (* ünïcode *) ".len();
    let location = snapshot.location(main, sum).expect("location");
    assert_eq!(location.line, line_number(&overlay, sum));
    assert_eq!(location.column, scalar_column(&overlay, sum));
    let byte_column = sum - line_start(&overlay, sum) + 1;
    assert!(
        byte_column > location.column as usize,
        "the non-ASCII comment is longer in bytes than in scalars"
    );

    let definition = snapshot.definition(main, sum).expect("definition");
    let declaration = find(&overlay, "  sum : DINT;") + "  ".len();
    let expected = snapshot
        .location(main, declaration)
        .expect("declaration location");
    assert_eq!(definition.line, expected.line);
    assert_eq!(definition.column, expected.column);

    let string_line = "  lamp := 'héllo' = 'héllo';";
    let overlay = source.replacen("    lamp := NOT lamp;", string_line, 1);
    assert_ne!(overlay, source);
    snapshot.set_overlay(&path, overlay.clone());
    let second_literal = overlay.rfind("'héllo'").expect("second literal");
    let location = snapshot.location(main, second_literal).expect("location");
    assert_eq!(location.column, scalar_column(&overlay, second_literal));
    let byte_column = second_literal - line_start(&overlay, second_literal) + 1;
    assert!(byte_column > location.column as usize);

    // Offsets past the end clamp to the final line.
    let end = snapshot
        .location(main, usize::MAX)
        .expect("clamped location");
    assert_eq!(end.line, line_number(&overlay, overlay.len()));
    assert_eq!(end.column, scalar_column(&overlay, overlay.len()));
}

fn line_start(source: &str, offset: usize) -> usize {
    source[..offset].rfind('\n').map_or(0, |index| index + 1)
}

fn scalar_column(source: &str, offset: usize) -> u32 {
    u32::try_from(source[line_start(source, offset)..offset].chars().count()).expect("column fits")
        + 1
}

fn line_number(source: &str, offset: usize) -> u32 {
    u32::try_from(source[..offset].matches('\n').count()).expect("line fits") + 1
}

#[test]
fn load_errors_are_reported() {
    let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/missing/stc.toml");
    let error = Snapshot::load(&missing).expect_err("a missing manifest fails");
    assert!(
        error
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code == "STC-E-PROJECT-001")
    );
    assert!(error.to_string().contains("STC-E-PROJECT-001"));
}
