//! Shared rename engine tests (D-253, WL-18).
//!
//! The tests load the committed examples in place and build temporary projects
//! for manifest, profile, and `VAR_ACCESS` coverage. Nothing is written.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stc_ide::{
    ColdReason, RenameError, RenameFile, RenameHistoryEntry, RenameOnline, RenameResult,
    RenameTargetKind, RenameWarning, Snapshot,
};
use stc_span::FileId;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

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

fn source_spellings(snapshot: &Snapshot, result: &RenameResult) -> Vec<(String, String, String)> {
    result
        .edits
        .iter()
        .filter_map(|edit| {
            let RenameFile::Source(file) = edit.file else {
                return None;
            };
            let info = snapshot.file(file).expect("source file");
            let text = snapshot.source(file).expect("source text");
            Some((
                info.display.clone(),
                text[edit.start as usize..edit.end as usize].to_owned(),
                edit.new_text.clone(),
            ))
        })
        .collect()
}

fn manifest_spellings(snapshot: &Snapshot, result: &RenameResult) -> Vec<(String, String)> {
    let file = snapshot.manifest_file();
    let text = snapshot
        .manifest_sources()
        .file(file)
        .expect("manifest source")
        .text();
    result
        .edits
        .iter()
        .filter(|edit| edit.file == RenameFile::Manifest)
        .map(|edit| {
            (
                text[edit.start as usize..edit.end as usize].to_owned(),
                edit.new_text.clone(),
            )
        })
        .collect()
}

fn profile_spellings(snapshot: &Snapshot, result: &RenameResult) -> Vec<(String, String)> {
    let Some(file) = snapshot.profile_file() else {
        return Vec::new();
    };
    let text = snapshot
        .profile_sources()
        .file(file)
        .expect("profile source")
        .text();
    result
        .edits
        .iter()
        .filter(|edit| edit.file == RenameFile::Profile)
        .map(|edit| {
            (
                text[edit.start as usize..edit.end as usize].to_owned(),
                edit.new_text.clone(),
            )
        })
        .collect()
}

fn assert_clean(snapshot: &Snapshot) {
    for info in snapshot.files() {
        let diagnostics = snapshot.diagnostics(info.id);
        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics in {}: {diagnostics:?}",
            info.display
        );
    }
}

struct TempProject {
    root: PathBuf,
}

impl TempProject {
    fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "stc-ide-rename-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp project");
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write test file");
        path
    }

    fn load(&self) -> Snapshot {
        Snapshot::load(&self.root).expect("snapshot loads")
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn basic_manifest(configuration: Option<&str>) -> String {
    let mut text = String::from(
        "schema = 1\n\n[package]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n",
    );
    if let Some(configuration) = configuration {
        text.push_str(&format!("\nconfiguration = \"{configuration}\"\n"));
    }
    text
}

#[test]
fn function_rename_is_multi_file_and_tracked() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    assert_clean(&snapshot);
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "sum := AddStep(sum)") + "sum := ".len();

    let prepared = snapshot
        .prepare_rename(main, offset)
        .expect("prepare succeeds")
        .expect("renameable target");
    assert_eq!(prepared.name, "AddStep");
    assert_eq!(
        &source[prepared.span.start as usize..prepared.span.end as usize],
        "AddStep"
    );

    let result = snapshot
        .rename(main, offset, "Advance")
        .expect("rename succeeds");
    assert_eq!(result.target, RenameTargetKind::Tracked);
    assert_eq!(result.online, RenameOnline::Preserved);
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "AddStep".into(),
            new: "Advance".into(),
        }]
    );
    assert!(result.warnings.is_empty());
    let spellings = source_spellings(&snapshot, &result);
    assert!(!spellings.is_empty());
    assert!(
        spellings
            .iter()
            .all(|(_, old, new)| old == "AddStep" && new == "Advance")
    );
    let files: Vec<&str> = spellings.iter().map(|(file, _, _)| file.as_str()).collect();
    assert!(files.iter().any(|file| file.ends_with("src/main.st")));
    assert!(files.iter().any(|file| file.ends_with("tests/smoke.st")));
    assert!(manifest_spellings(&snapshot, &result).is_empty());
}

#[test]
fn field_rename_uses_owner_qualified_history_and_named_arguments() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "increment : DINT");

    let result = snapshot
        .rename(main, offset, "delta")
        .expect("field rename succeeds");
    assert_eq!(result.target, RenameTargetKind::Tracked);
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Accumulator.increment".into(),
            new: "Accumulator.delta".into(),
        }]
    );
    let spellings = source_spellings(&snapshot, &result);
    assert!(
        spellings
            .iter()
            .all(|(_, old, new)| old == "increment" && new == "delta")
    );
    assert!(
        spellings
            .iter()
            .any(|(file, _, _)| file.ends_with("tests/smoke.st")),
        "the named argument labels are references: {spellings:?}"
    );
}

#[test]
fn configuration_rename_edits_the_manifest_and_is_history_free() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "CONFIGURATION Sample") + "CONFIGURATION ".len();

    let result = snapshot
        .rename(main, offset, "SampleX")
        .expect("configuration rename succeeds");
    assert_eq!(result.target, RenameTargetKind::HistoryFree);
    assert_eq!(
        result.online,
        RenameOnline::Cold(ColdReason::ConfigurationTaskGuard)
    );
    assert!(result.history.is_empty());
    assert_eq!(
        manifest_spellings(&snapshot, &result),
        vec![("Sample".to_owned(), "SampleX".to_owned())]
    );
    assert!(
        source_spellings(&snapshot, &result)
            .iter()
            .any(|(_, old, new)| old == "Sample" && new == "SampleX")
    );
}

#[test]
fn parameters_and_locals_are_history_free() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");

    let parameter = find(source, "value : DINT");
    let result = snapshot
        .rename(main, parameter, "input")
        .expect("parameter rename succeeds");
    assert_eq!(result.target, RenameTargetKind::HistoryFree);
    assert_eq!(result.online, RenameOnline::Preserved);
    assert!(result.history.is_empty());

    let local = find(source, "incremented : DINT");
    let result = snapshot
        .rename(main, local, "incremented_value")
        .expect("local rename succeeds");
    assert_eq!(result.target, RenameTargetKind::HistoryFree);
    assert!(result.history.is_empty());
}

#[test]
fn case_only_rename_has_edits_but_no_history() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "FUNCTION AddStep") + "FUNCTION ".len();

    let result = snapshot
        .rename(main, offset, "addstep")
        .expect("case-only rename succeeds");
    assert_eq!(result.target, RenameTargetKind::HistoryFree);
    assert_eq!(result.online, RenameOnline::Preserved);
    assert!(result.history.is_empty());
    let spellings = source_spellings(&snapshot, &result);
    assert!(!spellings.is_empty());
    assert!(
        spellings
            .iter()
            .all(|(_, old, new)| old == "AddStep" && new == "addstep")
    );
}

#[test]
fn invalid_same_and_colliding_names_are_rejected() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "FUNCTION AddStep") + "FUNCTION ".len();

    assert_eq!(
        snapshot.rename(main, offset, "9bad"),
        Err(RenameError::InvalidName("9bad".into()))
    );
    assert_eq!(
        snapshot.rename(main, offset, "FUNCTION"),
        Err(RenameError::InvalidName("FUNCTION".into()))
    );
    assert_eq!(
        snapshot.rename(main, offset, "AddStep"),
        Err(RenameError::SameName)
    );
    assert_eq!(
        snapshot.rename(main, offset, "Accumulator"),
        Err(RenameError::Collision("Accumulator".into()))
    );
    assert_eq!(
        snapshot.rename(main, offset, "ACCUMULATOR"),
        Err(RenameError::Collision("ACCUMULATOR".into()))
    );
    assert!(snapshot.rename(main, offset, "addstep").is_ok());
}

#[test]
fn non_identifier_and_unresolved_positions_are_not_renameable() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");

    assert!(snapshot.prepare_rename(main, 0).expect("prepare").is_none());
    assert_eq!(
        snapshot.rename(main, 0, "X"),
        Err(RenameError::NotAnIdentifier)
    );
    let literal = find(source, ":= 1;") + ":= ".len();
    assert!(
        snapshot
            .prepare_rename(main, literal)
            .expect("prepare")
            .is_none()
    );
}

#[test]
fn standard_functions_and_builtin_types_are_unsupported() {
    let snapshot = Snapshot::load(example_manifest("motor")).expect("motor loads");
    let motor = file_id(&snapshot, "src/motor.st");
    let source = snapshot.source(motor).expect("motor source");

    let len = find(source, "LEN(display)");
    assert_eq!(
        snapshot.prepare_rename(motor, len),
        Err(RenameError::Unsupported)
    );
    assert_eq!(
        snapshot.rename(motor, len, "LENGTH"),
        Err(RenameError::Unsupported)
    );
}

#[test]
fn dependency_declarations_are_rejected() {
    let snapshot = Snapshot::load(example_manifest("motor")).expect("motor loads");
    let motor = file_id(&snapshot, "src/motor.st");
    let source = snapshot.source(motor).expect("motor source");
    let drive = find(source, "drive : Drive") + "drive : ".len();

    assert_eq!(
        snapshot.prepare_rename(motor, drive),
        Err(RenameError::DependencyDeclaration)
    );
    assert_eq!(
        snapshot.rename(motor, drive, "MotorDrive"),
        Err(RenameError::DependencyDeclaration)
    );
}

#[test]
fn var_access_and_var_config_path_segments_are_edited() {
    let project = TempProject::new("access");
    project.write(
        "stc.toml",
        "schema = 1\n\n[package]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\nconfiguration = \"C\"\n",
    );
    let source = "TYPE Inner : STRUCT\n  x : DINT;\nEND_STRUCT; END_TYPE\nPROGRAM Main\nVAR\n  x : DINT;\n  nested : Inner;\nEND_VAR\nVAR_ACCESS\n  visible : x : DINT READ_ONLY;\n  nested_path : nested.x : DINT READ_ONLY;\nEND_VAR\n  x := 1;\nEND_PROGRAM\nCONFIGURATION C\nVAR_GLOBAL\n  seed : DINT;\nEND_VAR\nRESOURCE R ON CPU\n  TASK t (INTERVAL := T#10ms);\n  PROGRAM inst WITH t : Main;\nEND_RESOURCE\nVAR_ACCESS\n  seed_path : seed : DINT READ_WRITE;\n  inst_x : inst.x : DINT READ_ONLY;\n  inst_nested : inst.nested.x : DINT READ_ONLY;\nEND_VAR\nVAR_CONFIG\n  inst.nested.x AT %MD10 : DINT := 5;\nEND_VAR\nEND_CONFIGURATION\n";
    let path = project.write("src/main.st", source);
    let snapshot = project.load();
    assert_clean(&snapshot);
    let file = snapshot.file_id_for_path(&path).expect("main source id");

    let prepared = snapshot
        .prepare_rename(file, find(source, "visible : x") + "visible : ".len())
        .expect("prepare")
        .expect("access segment");
    assert_eq!(prepared.name, "x");
    assert_eq!(
        &source[prepared.span.start as usize..prepared.span.end as usize],
        "x"
    );

    let result = snapshot
        .rename(
            file,
            find(source, "visible : x") + "visible : ".len(),
            "value",
        )
        .expect("program field rename");
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Main.x".into(),
            new: "Main.value".into(),
        }]
    );
    let spellings = source_spellings(&snapshot, &result);
    assert_eq!(spellings.len(), 4, "{spellings:?}");
    assert!(
        spellings
            .iter()
            .all(|(_, old, new)| old == "x" && new == "value")
    );

    let result = snapshot
        .rename(file, find(source, "nested : Inner"), "inner")
        .expect("nested field rename");
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Main.nested".into(),
            new: "Main.inner".into(),
        }]
    );
    assert_eq!(source_spellings(&snapshot, &result).len(), 4);

    let result = snapshot
        .rename(file, find(source, "seed : DINT"), "start")
        .expect("global rename");
    assert_eq!(result.history.len(), 1);
    assert_eq!(result.history[0].old, "C.seed");
    assert_eq!(result.history[0].new, "C.start");
    assert_eq!(source_spellings(&snapshot, &result).len(), 2);

    let inner_x = find(source, "TYPE Inner")
        + source[find(source, "TYPE Inner")..]
            .find("x : DINT")
            .expect("inner x");
    let result = snapshot
        .rename(file, inner_x, "y")
        .expect("struct field rename");
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Inner.x".into(),
            new: "Inner.y".into(),
        }]
    );
    assert_eq!(source_spellings(&snapshot, &result).len(), 4);

    let config_x = find(source, "inst.nested.x AT") + "inst.nested.".len();
    let prepared = snapshot
        .prepare_rename(file, config_x)
        .expect("prepare")
        .expect("VAR_CONFIG segment");
    assert_eq!(prepared.name, "x");
    let result = snapshot
        .rename(file, config_x, "y")
        .expect("VAR_CONFIG field rename");
    assert_eq!(result.history[0].old, "Inner.x");
}

#[test]
fn fb_type_rename_is_tracked_and_multi_file() {
    let snapshot = Snapshot::load(sample_manifest()).expect("sample loads");
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "FUNCTION_BLOCK Accumulator") + "FUNCTION_BLOCK ".len();

    let result = snapshot
        .rename(main, offset, "Acc")
        .expect("FB type rename succeeds");
    assert_eq!(result.target, RenameTargetKind::Tracked);
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Accumulator".into(),
            new: "Acc".into(),
        }]
    );
    let spellings = source_spellings(&snapshot, &result);
    assert!(spellings.len() >= 3, "{spellings:?}");
    assert!(
        spellings
            .iter()
            .all(|(_, old, new)| old == "Accumulator" && new == "Acc")
    );
    assert!(
        spellings
            .iter()
            .any(|(file, _, _)| file.ends_with("tests/smoke.st"))
    );
}

#[test]
fn unresolved_identifiers_are_not_renameable() {
    let project = TempProject::new("unresolved");
    project.write("stc.toml", &basic_manifest(None));
    project.write(
        "src/main.st",
        "PROGRAM Main\nVAR\n  x : DINT;\nEND_VAR\n  x := MISSING;\nEND_PROGRAM\n",
    );
    let snapshot = project.load();
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let offset = find(source, "MISSING");
    assert!(
        snapshot
            .prepare_rename(main, offset)
            .expect("prepare")
            .is_none()
    );
    assert_eq!(
        snapshot.rename(main, offset, "OTHER"),
        Err(RenameError::Unresolved)
    );
}

#[test]
fn manifest_io_bindings_test_cases_and_profile_routes_are_edited() {
    let project = TempProject::new("external");
    project.write(
        "stc.toml",
        "schema = 1\n\n[package]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\nconfiguration = \"MainConfig\"\n\n[io]\nconfig = \"io/bench.toml\"\n\n[[io.devices]]\nname = \"bench\"\nkind = \"demo\"\n\n[[io.bindings]]\nvariable = \"Short\"\ndevice = \"bench\"\nchannel = 0\n\n[[io.bindings]]\nvariable = \"Ns\"\ndevice = \"bench\"\nchannel = 1\n\n[[io.bindings]]\nvariable = \"app.Pkg\"\ndevice = \"bench\"\nchannel = 2\n\n[[io.bindings]]\nvariable = \"main.Field\"\ndevice = \"bench\"\nchannel = 3\n\n[test]\nsources = [\"tests/**/*.st\"]\n\n[[test.case]]\nname = \"SampleTest\"\n",
    );
    project.write(
        "io/bench.toml",
        "[[routes]]\nsource = { kind = \"plc-output\", variable = \"Short\" }\nsinks = [{ kind = \"plc-input\", variable = \"Ns\" }]\n",
    );
    project.write(
        "src/main.st",
        "VAR_GLOBAL\n  Short AT %IX0.0 : BOOL;\n  Ns AT %IX0.1 : BOOL;\n  Pkg AT %IX0.2 : BOOL;\nEND_VAR\n\nPROGRAM Main\nVAR\n  Field AT %QX0.1 : BOOL;\nEND_VAR\n  Field := TRUE;\nEND_PROGRAM\n\nCONFIGURATION MainConfig\nRESOURCE Cpu ON Cpu\n  TASK MainTask(INTERVAL := T#10ms, PRIORITY := 1);\n  PROGRAM main WITH MainTask : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n",
    );
    project.write(
        "tests/sample_test.st",
        "{stc:test}\nFUNCTION SampleTest : BOOL\nVAR\n  result : BOOL;\nEND_VAR\n  result := TRUE;\n  SampleTest := result;\nEND_FUNCTION\n",
    );
    let snapshot = project.load();
    assert_clean(&snapshot);
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");

    let result = snapshot
        .rename(main, find(source, "Short AT"), "Brief")
        .expect("short binding rename");
    assert_eq!(result.history[0].old, "Short");
    assert_eq!(
        manifest_spellings(&snapshot, &result),
        vec![("Short".to_owned(), "Brief".to_owned())]
    );
    assert_eq!(
        profile_spellings(&snapshot, &result),
        vec![("Short".to_owned(), "Brief".to_owned())]
    );

    let result = snapshot
        .rename(main, find(source, "Ns AT"), "Node")
        .expect("short binding rename");
    assert_eq!(result.history[0].old, "Ns");
    assert_eq!(
        manifest_spellings(&snapshot, &result),
        vec![("Ns".to_owned(), "Node".to_owned())]
    );
    assert_eq!(
        profile_spellings(&snapshot, &result),
        vec![("Ns".to_owned(), "Node".to_owned())]
    );

    let result = snapshot
        .rename(main, find(source, "Pkg AT"), "Bundle")
        .expect("package-prefixed binding rename");
    assert_eq!(result.history[0].old, "Pkg");
    assert_eq!(
        manifest_spellings(&snapshot, &result),
        vec![("Pkg".to_owned(), "Bundle".to_owned())]
    );

    let result = snapshot
        .rename(main, find(source, "Field AT"), "Out")
        .expect("program field binding rename");
    assert_eq!(result.history[0].old, "Main.Field");
    assert_eq!(
        manifest_spellings(&snapshot, &result),
        vec![("Field".to_owned(), "Out".to_owned())]
    );

    let tests = file_id(&snapshot, "tests/sample_test.st");
    let test_source = snapshot.source(tests).expect("test source");
    let result = snapshot
        .rename(
            tests,
            find(test_source, "FUNCTION SampleTest") + "FUNCTION ".len(),
            "SmokeTest",
        )
        .expect("test POU rename");
    assert_eq!(result.history[0].old, "SampleTest");
    assert_eq!(
        manifest_spellings(&snapshot, &result),
        vec![("SampleTest".to_owned(), "SmokeTest".to_owned())]
    );
}

#[test]
fn a_missing_profile_warns_and_skips_profile_edits() {
    let project = TempProject::new("profile-missing");
    project.write(
        "stc.toml",
        "schema = 1\n\n[package]\nname = \"app\"\nversion = \"0.1.0\"\nlanguage = \"stc-2013-oop-utf8-v1\"\n\n[project]\nsources = [\"src/**/*.st\"]\n\n[io]\nconfig = \"io/missing.toml\"\n",
    );
    project.write(
        "src/main.st",
        "PROGRAM Main\nVAR\n  counter : DINT;\nEND_VAR\n  counter := counter + 1;\nEND_PROGRAM\n",
    );
    let snapshot = project.load();
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let result = snapshot
        .rename(main, find(source, "counter : DINT"), "count")
        .expect("rename proceeds without the profile");
    assert!(result.warnings.iter().any(|warning| {
        matches!(warning, RenameWarning::ProfileUnavailable(message) if message.contains("missing.toml"))
    }));
    assert!(!source_spellings(&snapshot, &result).is_empty());
    assert!(profile_spellings(&snapshot, &result).is_empty());
}

#[test]
fn inherited_member_conflicts_are_rejected_by_reanalysis() {
    let project = TempProject::new("inherited");
    project.write("stc.toml", &basic_manifest(None));
    project.write(
        "src/main.st",
        "FUNCTION_BLOCK Base\nMETHOD PUBLIC Start : BOOL\n  Start := TRUE;\nEND_METHOD\nEND_FUNCTION_BLOCK\n\nFUNCTION_BLOCK Child EXTENDS Base\nMETHOD PUBLIC Stop : BOOL\n  Stop := TRUE;\nEND_METHOD\nEND_FUNCTION_BLOCK\n\nPROGRAM Main\nVAR\n  child : Child;\nEND_VAR\n  child();\nEND_PROGRAM\n",
    );
    let snapshot = project.load();
    assert_clean(&snapshot);
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let result = snapshot.rename(main, find(source, "Stop : BOOL"), "Start");
    assert!(
        matches!(result, Err(RenameError::Analysis(_))),
        "expected a reanalysis rejection, got {result:?}"
    );
}

#[test]
fn enum_values_and_namespace_prefixes_use_canonical_history() {
    let project = TempProject::new("history");
    project.write("stc.toml", &basic_manifest(None));
    project.write(
        "src/main.st",
        "TYPE Mode : (Idle, Run); END_TYPE\n\nNAMESPACE Utils\nFUNCTION Helper : DINT\n  Helper := 1;\nEND_FUNCTION\nEND_NAMESPACE\n\nPROGRAM Main\nVAR\n  mode : Mode;\n  result : DINT;\nEND_VAR\n  mode := Run;\n  result := Utils.Helper();\nEND_PROGRAM\n",
    );
    let snapshot = project.load();
    assert_clean(&snapshot);
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");

    let result = snapshot
        .rename(main, find(source, "Run); END_TYPE"), "Running")
        .expect("enum value rename");
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Mode.Run".into(),
            new: "Mode.Running".into(),
        }]
    );

    let result = snapshot
        .rename(
            main,
            find(source, "NAMESPACE Utils") + "NAMESPACE ".len(),
            "Utility",
        )
        .expect("namespace rename");
    assert_eq!(
        result.history,
        vec![RenameHistoryEntry {
            old: "Utils".into(),
            new: "Utility".into(),
        }]
    );
    let spellings = source_spellings(&snapshot, &result);
    assert_eq!(spellings.len(), 2, "{spellings:?}");

    let helper = find(source, "Utils.Helper()") + "Utils.".len();
    let result = snapshot
        .rename(main, helper, "Assist")
        .expect("qualified function rename");
    assert_eq!(result.history[0].old, "Utils.Helper");
    let spellings = source_spellings(&snapshot, &result);
    assert_eq!(
        spellings
            .iter()
            .filter(|(_, old, new)| old == "Helper" && new == "Assist")
            .count(),
        3,
        "the declaration, the result assignment, and the qualified call are edited once each: {spellings:?}"
    );
    assert_eq!(
        spellings
            .iter()
            .filter(|(_, old, _)| old == "Helper")
            .count(),
        spellings.len(),
        "the reference edit and prefix edit are deduplicated"
    );
}

#[test]
fn a_non_selected_configuration_is_history_free_and_preserved() {
    let project = TempProject::new("configs");
    project.write("stc.toml", &basic_manifest(Some("First")));
    project.write(
        "src/main.st",
        "PROGRAM Main\nVAR\n  counter : DINT;\nEND_VAR\n  counter := 1;\nEND_PROGRAM\n\nCONFIGURATION First\nRESOURCE Cpu ON Cpu\n  TASK FirstTask(INTERVAL := T#10ms);\n  PROGRAM first WITH FirstTask : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n\nCONFIGURATION Second\nRESOURCE Cpu ON Cpu\n  TASK SecondTask(INTERVAL := T#10ms);\n  PROGRAM second WITH SecondTask : Main;\nEND_RESOURCE\nEND_CONFIGURATION\n",
    );
    let snapshot = project.load();
    assert_clean(&snapshot);
    let main = file_id(&snapshot, "src/main.st");
    let source = snapshot.source(main).expect("main source");
    let result = snapshot
        .rename(
            main,
            find(source, "CONFIGURATION Second") + "CONFIGURATION ".len(),
            "Third",
        )
        .expect("non-selected configuration rename");
    assert_eq!(result.target, RenameTargetKind::HistoryFree);
    assert_eq!(result.online, RenameOnline::Preserved);
    assert!(result.history.is_empty());
    assert!(manifest_spellings(&snapshot, &result).is_empty());
}
