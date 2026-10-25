//! Thin executable launcher for a loadable module pair.
//!
//! The launcher contains no generated program code. Its `main` passes the
//! adjacent module file name to the host runtime, which owns the cycle loop,
//! the GOT, the process image, and the module lifecycle (Runtime §1.1).

use crate::abi;
use crate::ir::{Function, GlobalDef, IrType, Module, SymbolAttrs, Value, c_string};

/// Name of the module-relative file-name global.
pub const MODULE_PATH: &str = "stc_module_path";

/// Generates the launcher IR for a module file.
///
/// `module_file` is the file name the runtime resolves relative to the
/// launcher's directory.
#[must_use]
pub fn generate_launcher(artifact_name: &str, module_file: &str, target: Option<&str>) -> String {
    let mut module = Module::new(artifact_name, target);
    module.add_global(GlobalDef {
        name: MODULE_PATH.into(),
        attrs: SymbolAttrs::private(),
        ty: format!("[{} x i8]", module_file.len() + 1),
        init: c_string(module_file.as_bytes()),
        constant: true,
        align: Some(1),
    });
    module.declare(abi::symbols::EXECUTE, "i32 @stc_rt_execute(i32, ptr, ptr)");

    let mut f = Function::new(
        "main",
        SymbolAttrs::external(),
        IrType::Int(32),
        vec![
            (IrType::Int(32), "%argc".into()),
            (IrType::Ptr, "%argv".into()),
        ],
    );
    f.start_fresh_block();
    let argc = f.param_value(0);
    let argv = f.param_value(1);
    let code = f.call(
        IrType::Int(32),
        abi::symbols::EXECUTE,
        &[argc, argv, Value::symbol(MODULE_PATH)],
    );
    match code {
        Some(code) => f.ret(Some(&code)),
        None => f.ret(Some(&Value::int(32, 1))),
    }
    debug_assert!(f.check().is_ok());
    module.add_function(f);
    module.render()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_is_deterministic_and_names_the_module() {
        let first = generate_launcher("demo", "demo-abc.dll", None);
        let second = generate_launcher("demo", "demo-abc.dll", None);
        assert_eq!(first, second);
        assert!(
            first.contains("define i32 @main(i32 %argc, ptr %argv)"),
            "{first}"
        );
        assert!(
            first.contains("call i32 @stc_rt_execute(i32 %argc, ptr %argv, ptr @stc_module_path)"),
            "{first}"
        );
        assert!(first.contains("c\"demo-abc.dll\\00\""), "{first}");
        assert!(!first.contains("stc_module_cycle"), "{first}");
    }
}
