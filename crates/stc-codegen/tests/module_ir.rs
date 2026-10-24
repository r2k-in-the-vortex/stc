//! Phase-3 module-IR builder and structural-verifier tests.
//!
//! The fixture assembles the IR shape a generated M4.5 module will have:
//! a private descriptor blob, a function-address table, cached slot-table and
//! host-API pointers, the ABI 2.0 entry points, a data-root cell, a function
//! cell, and an import cell. Generated code never names the root or the user
//! callable; both are reached through cell values.

use stc_codegen::abi;
use stc_codegen::ir::{
    CellKind, Function, GlobalDef, IrType, Linkage, Module, SymbolAttrs, Value, Visibility,
    byte_string,
};
use stc_codegen::verify::{Contract, DescriptorSpec, FunctionTableSpec, verify};

const ROOT_COUNTER: &str = "stc_g_counter";
const USER_ADD: &str = "stc_user_add";
const DESCRIPTOR: &str = "stc_module_descriptor_data";
const FUNCTION_TABLE: &str = "stc_module_function_table";
const SLOTS: &str = "stc_module_slots";
const API: &str = "stc_module_api";

const DESCRIPTOR_BYTES: &[u8] = &[
    b'S', b'T', b'C', b'M', b'O', b'D', 0, 0, 1, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0,
];

fn local_global(name: &str) -> GlobalDef {
    GlobalDef {
        name: name.into(),
        attrs: SymbolAttrs::internal(),
        ty: "ptr".into(),
        init: "null".into(),
        constant: false,
        align: None,
    }
}

fn function(name: &str, attrs: SymbolAttrs, ret: IrType, params: &[(&str, IrType)]) -> Function {
    let params = params
        .iter()
        .map(|(name, ty)| (*ty, (*name).to_owned()))
        .collect();
    Function::new(name, attrs, ret, params)
}

fn module_fixture() -> Module {
    let mut module = Module::new("fixture", None);
    module.add_descriptor(DESCRIPTOR, DESCRIPTOR_BYTES);
    module.add_function_table(FUNCTION_TABLE, &[USER_ADD]);
    module.add_global(local_global(SLOTS));
    module.add_global(local_global(API));

    let mut descriptor = function(
        abi::module::DESCRIPTOR,
        SymbolAttrs::module_export(),
        IrType::Ptr,
        &[("%out_size", IrType::Ptr)],
    );
    descriptor.start_fresh_block();
    let out_size = descriptor.param_value(0);
    descriptor.store(
        &Value::int(64, DESCRIPTOR_BYTES.len() as i128),
        &out_size,
        Some(8),
    );
    descriptor.ret(Some(&Value::symbol(DESCRIPTOR)));
    module.add_function(descriptor);

    let mut functions = function(
        abi::module::FUNCTIONS,
        SymbolAttrs::module_export(),
        IrType::Ptr,
        &[("%out_count", IrType::Ptr)],
    );
    functions.start_fresh_block();
    let out_count = functions.param_value(0);
    functions.store(&Value::int(32, 1), &out_count, Some(4));
    functions.ret(Some(&Value::symbol(FUNCTION_TABLE)));
    module.add_function(functions);

    let mut attach = function(
        abi::module::ATTACH,
        SymbolAttrs::module_export(),
        IrType::Int(32),
        &[
            ("%api", IrType::Ptr),
            ("%slots", IrType::Ptr),
            ("%slot_count", IrType::Int(32)),
            ("%generation", IrType::Int(64)),
        ],
    );
    attach.start_fresh_block();
    let api = attach.param_value(0);
    let slots = attach.param_value(1);
    attach.store(&api, &Value::symbol(API), Some(8));
    attach.store(&slots, &Value::symbol(SLOTS), Some(8));
    attach.ret(Some(&Value::int(32, 0)));
    module.add_function(attach);

    let mut prepare = function(
        abi::module::PREPARE,
        SymbolAttrs::module_export(),
        IrType::Int(32),
        &[
            ("%mode", IrType::Int(32)),
            ("%generation", IrType::Int(64)),
            ("%fresh_mask", IrType::Ptr),
            ("%mask_size", IrType::Int(32)),
        ],
    );
    prepare.start_fresh_block();
    prepare.ret(Some(&Value::int(32, 0)));
    module.add_function(prepare);

    let mut cycle = function(
        abi::module::CYCLE,
        SymbolAttrs::module_export(),
        IrType::Int(32),
        &[("%generation", IrType::Int(64))],
    );
    cycle.start_fresh_block();
    let slots = cycle.load(IrType::Ptr, &Value::symbol(SLOTS), Some(8));
    let counter = cycle.slot_value(&slots, 0);
    let value = cycle.load(IrType::Int(32), &counter, Some(4));
    let next = cycle.binop("add", &value, &Value::int(32, 1));
    cycle.store(&next, &counter, Some(4));
    let callee = cycle.slot_value(&slots, 1);
    let _result = cycle.call_ptr(IrType::Int(32), &callee, std::slice::from_ref(&next));
    let clock = cycle.slot_value(&slots, 2);
    let _now = cycle.call_ptr(IrType::Int(64), &clock, &[]);
    cycle.ret(Some(&Value::int(32, 0)));
    module.add_function(cycle);

    let mut shutdown = function(
        abi::module::SHUTDOWN,
        SymbolAttrs::module_export(),
        IrType::Int(32),
        &[("%mode", IrType::Int(32))],
    );
    shutdown.start_fresh_block();
    shutdown.ret(Some(&Value::int(32, 0)));
    module.add_function(shutdown);

    let mut detach = function(
        abi::module::DETACH,
        SymbolAttrs::module_export(),
        IrType::Void,
        &[("%generation", IrType::Int(64))],
    );
    detach.start_fresh_block();
    detach.ret(None);
    module.add_function(detach);

    let mut user_add = function(
        USER_ADD,
        SymbolAttrs::internal(),
        IrType::Int(32),
        &[("%value", IrType::Int(32))],
    );
    user_add.start_fresh_block();
    let value = user_add.param_value(0);
    let sum = user_add.binop("add", &value, &Value::int(32, 1));
    user_add.ret(Some(&sum));
    module.add_function(user_add);

    module
}

fn fixture_contract() -> Contract {
    let mut contract =
        Contract::for_module(vec![CellKind::Data, CellKind::Function, CellKind::Import]);
    contract.roots.insert(ROOT_COUNTER.into());
    contract.functions.insert(USER_ADD.into());
    contract.local_globals.insert(SLOTS.into());
    contract.local_globals.insert(API.into());
    contract.descriptor = Some(DescriptorSpec {
        symbol: DESCRIPTOR.into(),
        size: DESCRIPTOR_BYTES.len(),
    });
    contract.function_table = Some(FunctionTableSpec {
        symbol: FUNCTION_TABLE.into(),
        count: 1,
    });
    contract
}

fn rules(module: &Module, contract: &Contract) -> Vec<&'static str> {
    match verify(module, contract) {
        Ok(()) => Vec::new(),
        Err(errors) => errors.into_iter().map(|error| error.rule).collect(),
    }
}

#[test]
fn module_ir_satisfies_the_got_contract() {
    let module = module_fixture();
    if let Err(errors) = verify(&module, &fixture_contract()) {
        panic!(
            "fixture violates the module contract:\n{}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn slot_access_and_indirect_calls_render_canonically() {
    let module = module_fixture();
    let cycle = module.functions[abi::module::CYCLE].render();
    assert!(
        cycle.contains("getelementptr ptr, ptr %t0, i64 0"),
        "data slot access:\n{cycle}"
    );
    assert!(
        cycle.contains("getelementptr ptr, ptr %t0, i64 1"),
        "function slot access:\n{cycle}"
    );
    assert!(
        cycle.contains("getelementptr ptr, ptr %t0, i64 2"),
        "import slot access:\n{cycle}"
    );
    let loads = cycle
        .lines()
        .filter(|line| line.contains("load ptr, ptr %t"))
        .count();
    assert_eq!(loads, 3, "one cell load per slot:\n{cycle}");
    assert!(
        cycle
            .lines()
            .any(|line| line.contains("= call i32 %t6(i32 %t4)")),
        "function-cell call:\n{cycle}"
    );
    assert!(
        cycle.lines().any(|line| line.contains("= call i64 %t9()")),
        "import-cell call:\n{cycle}"
    );

    let rendered = module.render();
    assert!(
        rendered.contains("define dso_local dllexport ptr @stc_module_descriptor(ptr %out_size) {")
    );
    assert!(rendered.contains(&format!(
        "@{DESCRIPTOR} = private constant [{} x i8] {}, align 8",
        DESCRIPTOR_BYTES.len(),
        byte_string(DESCRIPTOR_BYTES)
    )));
    assert!(rendered.contains(&format!(
        "@{FUNCTION_TABLE} = internal constant [1 x ptr] [ptr @{USER_ADD}], align 8"
    )));
}

#[test]
fn module_ir_is_deterministic() {
    assert_eq!(module_fixture().render(), module_fixture().render());
}

#[test]
fn symbol_visibility_renders_in_llvm_order() {
    let attrs = |linkage, visibility, export, dso_local| SymbolAttrs {
        linkage,
        visibility,
        export,
        dso_local,
    };
    assert_eq!(
        attrs(Linkage::External, Visibility::Default, true, true).render(),
        "dso_local dllexport "
    );
    assert_eq!(
        attrs(Linkage::External, Visibility::Hidden, false, true).render(),
        "dso_local hidden "
    );
    assert_eq!(
        attrs(Linkage::External, Visibility::Protected, true, false).render(),
        "protected dllexport "
    );
    assert_eq!(
        attrs(Linkage::Internal, Visibility::Protected, false, false).render(),
        "internal "
    );
    assert_eq!(
        attrs(Linkage::Private, Visibility::Default, false, false).render(),
        "private "
    );
    assert_eq!(
        attrs(Linkage::External, Visibility::Default, false, false).render(),
        ""
    );
}

#[test]
fn direct_and_descriptor_cells_keep_their_kinds() {
    let mut module = Module::new("direct", None);
    let mut body = function(
        "stc_read_direct",
        SymbolAttrs::internal(),
        IrType::Void,
        &[("%slots", IrType::Ptr)],
    );
    body.start_fresh_block();
    let slots = body.param_value(0);
    let storage = body.slot_value(&slots, 0);
    let value = body.load(IrType::Int(32), &storage, Some(4));
    body.store(&value, &storage, Some(4));
    body.ret(None);
    module.add_function(body);

    let contract = Contract {
        slots: vec![CellKind::Direct, CellKind::Descriptor],
        ..Contract::default()
    };
    assert!(verify(&module, &contract).is_ok());
}

#[test]
fn module_ir_is_accepted_by_clang() {
    let module = module_fixture();
    assert!(verify(&module, &fixture_contract()).is_ok());
    verify_with_clang("module_fixture", &module.render());
}

#[test]
fn direct_root_definition_is_rejected() {
    let mut module = module_fixture();
    module.add_global(GlobalDef {
        name: ROOT_COUNTER.into(),
        attrs: SymbolAttrs::internal(),
        ty: "i32".into(),
        init: "0".into(),
        constant: false,
        align: None,
    });
    assert!(rules(&module, &fixture_contract()).contains(&"root-global"));
}

#[test]
fn direct_root_reference_is_rejected() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_root_read",
        SymbolAttrs::internal(),
        IrType::Void,
        &[],
    );
    body.start_fresh_block();
    let _ = body.load(IrType::Int(32), &Value::symbol(ROOT_COUNTER), Some(4));
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"direct-root-reference"));
}

#[test]
fn direct_global_access_must_be_an_allowlisted_module_global() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_global_read",
        SymbolAttrs::internal(),
        IrType::Void,
        &[],
    );
    body.start_fresh_block();
    let _ = body.load(IrType::Int(32), &Value::symbol("stc_user_data"), Some(4));
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"global-access"));
}

#[test]
fn initializer_references_are_checked() {
    let mut module = module_fixture();
    module.add_global(GlobalDef {
        name: "stc_bad_root_pointer".into(),
        attrs: SymbolAttrs::internal(),
        ty: "ptr".into(),
        init: format!("ptr @{ROOT_COUNTER}"),
        constant: true,
        align: Some(8),
    });
    assert!(rules(&module, &fixture_contract()).contains(&"direct-root-reference"));

    let mut module = module_fixture();
    module.add_global(GlobalDef {
        name: "stc_bad_function_pointer".into(),
        attrs: SymbolAttrs::internal(),
        ty: "ptr".into(),
        init: format!("ptr @{USER_ADD}"),
        constant: true,
        align: Some(8),
    });
    assert!(rules(&module, &fixture_contract()).contains(&"direct-function-reference"));
}

#[test]
fn direct_user_call_is_rejected() {
    let mut module = module_fixture();
    let mut body = function("stc_bad_direct", SymbolAttrs::internal(), IrType::Void, &[]);
    body.start_fresh_block();
    let _ = body.call(IrType::Int(32), USER_ADD, &[Value::int(32, 1)]);
    body.ret(None);
    module.add_function(body);
    let rules = rules(&module, &fixture_contract());
    assert!(rules.contains(&"direct-function-reference"));
    assert!(rules.contains(&"direct-user-call"));
}

#[test]
fn indirect_call_requires_a_cell_value() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_pointer",
        SymbolAttrs::internal(),
        IrType::Void,
        &[("%callee", IrType::Ptr)],
    );
    body.start_fresh_block();
    let callee = body.param_value(0);
    let _ = body.call_ptr(IrType::Void, &callee, &[]);
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"indirect-call"));
}

#[test]
fn call_through_a_data_cell_is_rejected() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_data_call",
        SymbolAttrs::internal(),
        IrType::Void,
        &[("%slots", IrType::Ptr)],
    );
    body.start_fresh_block();
    let slots = body.param_value(0);
    let storage = body.slot_value(&slots, 0);
    let _ = body.call_ptr(IrType::Void, &storage, &[]);
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"slot-kind"));
}

#[test]
fn storage_access_through_a_function_cell_is_rejected() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_function_load",
        SymbolAttrs::internal(),
        IrType::Void,
        &[("%slots", IrType::Ptr)],
    );
    body.start_fresh_block();
    let slots = body.param_value(0);
    let code = body.slot_value(&slots, 1);
    let _ = body.load(IrType::Int(32), &code, Some(4));
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"slot-kind"));
}

#[test]
fn writing_a_got_cell_is_rejected() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_cell_write",
        SymbolAttrs::internal(),
        IrType::Void,
        &[("%slots", IrType::Ptr)],
    );
    body.start_fresh_block();
    let slots = body.param_value(0);
    let cell = body.slot_cell(&slots, 0);
    body.store(&Value::null_ptr(), &cell, Some(8));
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"cell-write"));
}

#[test]
fn pre_attach_entry_points_must_not_use_the_slot_table() {
    let mut module = module_fixture();
    module.functions.remove(abi::module::DESCRIPTOR);
    let mut entry = function(
        abi::module::DESCRIPTOR,
        SymbolAttrs::module_export(),
        IrType::Ptr,
        &[("%out_size", IrType::Ptr)],
    );
    entry.start_fresh_block();
    let out_size = entry.param_value(0);
    let cell = entry.slot_cell(&out_size, 0);
    let _value = entry.cell_value(&cell);
    entry.store(&Value::int(64, 1), &out_size, Some(8));
    entry.ret(Some(&Value::symbol(DESCRIPTOR)));
    module.add_function(entry);
    assert!(rules(&module, &fixture_contract()).contains(&"pre-attach"));
}

#[test]
fn out_of_range_slots_are_rejected() {
    let mut module = module_fixture();
    let mut body = function(
        "stc_bad_slot",
        SymbolAttrs::internal(),
        IrType::Void,
        &[("%slots", IrType::Ptr)],
    );
    body.start_fresh_block();
    let slots = body.param_value(0);
    let storage = body.slot_value(&slots, 9);
    let _ = body.load(IrType::Int(32), &storage, Some(4));
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"slot-range"));
}

#[test]
fn missing_entry_point_is_rejected() {
    let mut module = module_fixture();
    module.functions.remove(abi::module::SHUTDOWN);
    assert!(rules(&module, &fixture_contract()).contains(&"entry-point"));
}

#[test]
fn wrong_entry_point_signature_is_rejected() {
    let mut module = module_fixture();
    module.functions.get_mut(abi::module::CYCLE).unwrap().ret = IrType::Ptr;
    assert!(rules(&module, &fixture_contract()).contains(&"entry-point"));
}

#[test]
fn descriptor_shape_is_checked() {
    let mut module = module_fixture();
    let mut contract = fixture_contract();
    contract.descriptor = Some(DescriptorSpec {
        symbol: DESCRIPTOR.into(),
        size: DESCRIPTOR_BYTES.len() + 1,
    });
    assert!(rules(&module, &contract).contains(&"descriptor"));

    let mut body = function(
        "stc_bad_descriptor_read",
        SymbolAttrs::internal(),
        IrType::Void,
        &[],
    );
    body.start_fresh_block();
    let _ = body.load(IrType::Int(8), &Value::symbol(DESCRIPTOR), Some(1));
    body.ret(None);
    module.add_function(body);
    assert!(rules(&module, &fixture_contract()).contains(&"descriptor"));
}

#[test]
fn function_table_shape_is_checked() {
    let module = module_fixture();
    let mut contract = fixture_contract();
    contract.function_table = Some(FunctionTableSpec {
        symbol: FUNCTION_TABLE.into(),
        count: 2,
    });
    assert!(rules(&module, &contract).contains(&"function-table"));
}

fn clang() -> Option<String> {
    let candidate = std::env::var("STC_CLANG").unwrap_or_else(|_| "clang".into());
    match std::process::Command::new(&candidate)
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => Some(candidate),
        _ => {
            assert!(
                std::env::var_os("STC_REQUIRE_CLANG").is_none(),
                "clang is required (STC_REQUIRE_CLANG=1) but was not found"
            );
            eprintln!("note: clang not found; skipping IR verification");
            None
        }
    }
}

fn verify_with_clang(name: &str, ir: &str) {
    let Some(clang) = clang() else {
        return;
    };
    let directory = std::env::temp_dir().join(format!("stc-codegen-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create temp directory");
    let path = directory.join(format!("{name}.ll"));
    std::fs::write(&path, ir).expect("write IR");
    for target in [
        None,
        Some("x86_64-pc-windows-msvc"),
        Some("x86_64-unknown-linux-gnu"),
    ] {
        let label = target.unwrap_or("host");
        let object = directory.join(format!("{name}-{label}.o"));
        let mut command = std::process::Command::new(&clang);
        if let Some(target) = target {
            command.arg(format!("--target={target}"));
        }
        let output = command
            .args(["-x", "ir", "-c", "-o"])
            .arg(&object)
            .arg(&path)
            .output()
            .expect("run clang");
        assert!(
            output.status.success(),
            "clang rejected {name} for {label}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _ = std::fs::remove_file(&object);
    }
    let _ = std::fs::remove_file(&path);
}
