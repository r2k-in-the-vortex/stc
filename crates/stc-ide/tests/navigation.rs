//! Regression tests for navigation shapes beyond simple value references:
//! named-argument labels, method-call receivers, interface lists,
//! configuration program types, and `CASE` labels.

use std::path::{Path, PathBuf};

use stc_ide::Snapshot;
use stc_span::FileId;

fn example_manifest(example: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples")
        .join(example)
        .join("stc.toml")
}

fn sample_manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../editors/vscode/samples/workspace/stc.toml")
}

fn file_id(snapshot: &Snapshot, suffix: &str) -> FileId {
    snapshot
        .files()
        .iter()
        .find(|info| info.display.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing source {suffix:?}"))
        .id
}

fn find(source: &str, needle: &str) -> usize {
    source
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?}"))
}

fn assert_definition_at(
    snapshot: &Snapshot,
    file: FileId,
    use_offset: usize,
    declaration_offset: usize,
    name: &str,
) {
    let definition = snapshot
        .definition(file, use_offset)
        .unwrap_or_else(|| panic!("no definition at offset {use_offset}"));
    assert_eq!(definition.file, file, "definition for {name:?}");
    assert_eq!(definition.span.start as usize, declaration_offset);
    assert_eq!(
        declaration_offset + name.len(),
        definition.span.end as usize
    );
}

#[test]
fn named_argument_labels_resolve_to_parameters() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let file = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(file).expect("source");

    let input_label = find(source, "increment := INCREMENT");
    let input_declaration = find(source, "increment : DINT");
    assert_definition_at(&snapshot, file, input_label, input_declaration, "increment");

    let output_label = find(source, "total => sum");
    let output_declaration = find(source, "total : DINT");
    assert_definition_at(&snapshot, file, output_label, output_declaration, "total");
}

#[test]
fn fb_instance_call_receiver_resolves_to_its_field() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let file = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(file).expect("source");
    let use_offset = find(source, "accumulator(increment");
    let declaration = find(source, "accumulator : Accumulator");
    assert_definition_at(&snapshot, file, use_offset, declaration, "accumulator");
}

#[test]
fn method_call_receivers_resolve_to_their_declaration() {
    let snapshot = Snapshot::load(example_manifest("motor")).expect("motor loads");
    let file = file_id(&snapshot, "motor/src/motor.st");
    let source = snapshot.source(file).expect("source");
    let declaration = find(source, "drive : Drive");

    let method_receiver = find(source, "drive.Start");
    assert_definition_at(&snapshot, file, method_receiver, declaration, "drive");

    let field_receiver = find(source, "drive.state");
    assert_definition_at(&snapshot, file, field_receiver, declaration, "drive");
}

#[test]
fn implements_interface_resolves_to_the_interface() {
    let snapshot = Snapshot::load(example_manifest("motorlib")).expect("motorlib loads");
    let file = file_id(&snapshot, "motorlib/src/motorlib.st");
    let source = snapshot.source(file).expect("source");
    let use_offset = find(source, "IMPLEMENTS IDrive") + "IMPLEMENTS ".len();
    let declaration = find(source, "INTERFACE IDrive") + "INTERFACE ".len();
    assert_definition_at(&snapshot, file, use_offset, declaration, "IDrive");
}

#[test]
fn configuration_program_type_resolves_to_the_program() {
    let snapshot = Snapshot::load(example_manifest("motor")).expect("motor loads");
    let file = file_id(&snapshot, "motor/src/motor.st");
    let source = snapshot.source(file).expect("source");
    let use_offset = find(source, "WITH MainTask : Motor") + "WITH MainTask : ".len();
    let declaration = find(source, "PROGRAM Motor") + "PROGRAM ".len();
    assert_definition_at(&snapshot, file, use_offset, declaration, "Motor");
}

#[test]
fn case_labels_resolve_to_enumeration_values() {
    let snapshot = Snapshot::load(example_manifest("motorlib")).expect("motorlib loads");
    let file = file_id(&snapshot, "motorlib/src/motorlib.st");
    let source = snapshot.source(file).expect("source");

    let idle_label = find(source, "Idle:");
    let idle_declaration = find(source, "Idle,");
    assert_definition_at(&snapshot, file, idle_label, idle_declaration, "Idle");

    let running_label = find(source, "Running:");
    let running_declaration = find(source, "Running,");
    assert_definition_at(
        &snapshot,
        file,
        running_label,
        running_declaration,
        "Running",
    );
}

#[test]
fn identifier_end_positions_resolve() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let file = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(file).expect("source");

    // The caret immediately before `(` must resolve the callee.
    let call_end = find(source, "accumulator(increment") + "accumulator".len();
    let call_declaration = find(source, "accumulator : Accumulator");
    assert_definition_at(&snapshot, file, call_end, call_declaration, "accumulator");

    let function_end = find(source, "AddStep(sum)") + "AddStep".len();
    let function_declaration = find(source, "FUNCTION AddStep") + "FUNCTION ".len();
    assert_definition_at(
        &snapshot,
        file,
        function_end,
        function_declaration,
        "AddStep",
    );
}

#[test]
fn standard_functions_have_no_source_definition() {
    let snapshot = Snapshot::load(example_manifest("motor")).expect("motor loads");
    let file = file_id(&snapshot, "motor/src/motor.st");
    let source = snapshot.source(file).expect("source");
    assert!(snapshot.definition(file, find(source, "LEN(")).is_none());
}
