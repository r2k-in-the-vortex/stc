//! Stable, C-identifier-safe symbol names and signature hashes (D-082).

use stc_hir::{ArgDirection, DefId, DefKind, HirCrate, Symbol};

use crate::CodegenError;
use crate::layout::{canonical_type_name, fnv1a};

/// Normalizes an identifier to uppercase and escapes non-C characters.
#[must_use]
pub fn normalize_ident(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || character == '_' {
            out.push(character.to_ascii_uppercase());
        } else {
            out.push_str(&format!("_u{:04X}_", u32::from(character)));
        }
    }
    out
}

/// Returns the normalized namespace segments of a definition, root omitted.
#[must_use]
pub fn namespace_segments(hir: &HirCrate, def: DefId) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = hir.defs[def.index()].parent;
    while let Some(parent) = current {
        let parent_def = &hir.defs[parent.index()];
        if !matches!(parent_def.kind, DefKind::Namespace { .. }) {
            break;
        }
        if parent_def.parent.is_some() {
            segments.push(normalize_ident(parent_def.name_text(&hir.interner)));
        }
        current = parent_def.parent;
    }
    segments.reverse();
    segments
}

fn prefix(prefix: &str, hir: &HirCrate, def: DefId, name: &str) -> String {
    let mut out = String::from(prefix);
    for segment in namespace_segments(hir, def) {
        out.push_str(&segment);
        out.push_str("_n_");
    }
    out.push_str(name);
    out
}

/// Returns the mangled symbol of a function.
///
/// # Errors
///
/// Returns `STC-E-CODEGEN-001` for non-scalar signatures.
pub fn function_symbol(hir: &HirCrate, def: DefId) -> Result<String, CodegenError> {
    let name = normalize_ident(hir.def_name(def));
    let signature = signature_text(hir, def)?;
    let hash = fnv1a(signature.as_bytes());
    Ok(format!(
        "{}_{hash:016x}",
        prefix("stc_fn_", hir, def, &name)
    ))
}

/// Returns the canonical signature text used by the hash.
///
/// # Errors
///
/// Returns `STC-E-CODEGEN-001` for non-scalar parameter/result types.
pub fn signature_text(hir: &HirCrate, def: DefId) -> Result<String, CodegenError> {
    let (result, params) = match &hir.defs[def.index()].kind {
        DefKind::Function {
            result,
            params,
            body: _,
        } => (*result, params.clone()),
        DefKind::Method { result, params, .. } => (*result, params.clone()),
        _ => (None, Vec::new()),
    };
    let mut out = String::from("(");
    for (index, param) in params.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let param = hir.param(*param);
        out.push_str(match param.direction {
            ArgDirection::Input => "IN:",
            ArgDirection::Output => "OUT:",
            ArgDirection::InOut => "INOUT:",
        });
        out.push_str(&canonical_type_name(hir, param.ty)?);
    }
    out.push_str(")->");
    match result {
        Some(result) => out.push_str(&canonical_type_name(hir, result)?),
        None => out.push_str("void"),
    }
    Ok(out)
}

/// Returns the mangled symbol of a global.
#[must_use]
pub fn global_symbol(hir: &HirCrate, def: DefId) -> String {
    prefix("stc_g_", hir, def, &normalize_ident(hir.def_name(def)))
}

/// Returns the LLVM struct name (without `%`) for a program type.
#[must_use]
pub fn program_type_name(hir: &HirCrate, def: DefId) -> String {
    let mut out = String::from("stc.prog.");
    for segment in namespace_segments(hir, def) {
        out.push_str(&segment);
        out.push_str("_n_");
    }
    out.push_str(&normalize_ident(hir.def_name(def)));
    out
}

/// Returns the program instance global name (without `@`).
#[must_use]
pub fn program_instance_symbol(name_text: &str) -> String {
    format!("stc_pi_{}", normalize_ident(name_text))
}

/// Returns the program body function symbol.
#[must_use]
pub fn program_body_symbol(hir: &HirCrate, def: DefId) -> String {
    let mut out = String::from("stc_prog_");
    for segment in namespace_segments(hir, def) {
        out.push_str(&segment);
        out.push_str("_n_");
    }
    out.push_str(&normalize_ident(hir.def_name(def)));
    out.push_str("_body");
    out
}

/// Returns the SINGLE-task guard global name.
#[must_use]
pub fn task_guard_symbol(resource: &str, name: Symbol, hir: &HirCrate) -> String {
    task_guard_symbol_text(resource, hir.interner.text(name))
}

/// Returns a task-guard symbol from already-resolved name text.
#[must_use]
pub fn task_guard_symbol_text(resource: &str, name: &str) -> String {
    format!(
        "stc_single_{}_{}",
        normalize_ident(resource),
        normalize_ident(name)
    )
}
