//! Committed Lean vector checks (Testing and Verification §5.2–§5.3).
//!
//! Every implemented exact function must have a generated vector file, every
//! vector case must agree with the Rust exact reference model in
//! [`stc_codegen::exact`], and the recorded generator digest must match the
//! file's cases. The constant-folding test checks that `stc-sema` folds the
//! same operations to the model's results.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use stc_codegen::exact::{self, Compare, FunctionKind, IntType, Res, StringKind, Trap};

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .join("tests/vectors/exact")
}

fn vector_files() -> Vec<(String, Value)> {
    let mut files: Vec<_> = std::fs::read_dir(vectors_dir())
        .expect("vector directory")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).expect("read vector");
            let value: Value = serde_json::from_str(&text)
                .unwrap_or_else(|error| panic!("invalid JSON in {}: {error}", path.display()));
            (
                path.file_name().unwrap().to_string_lossy().into_owned(),
                value,
            )
        })
        .collect()
}

fn object<'a>(value: &'a Value, context: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} is not a JSON object"))
}

fn field<'a>(case: &'a Map<String, Value>, key: &str) -> &'a Value {
    case.get(key)
        .unwrap_or_else(|| panic!("missing case field `{key}` in {case:?}"))
}

fn int_value(value: &Value) -> i128 {
    if let Some(number) = value.as_i64() {
        i128::from(number)
    } else if let Some(number) = value.as_u64() {
        i128::from(number)
    } else {
        panic!("expected an integer, found {value}")
    }
}

fn nat_value(value: &Value) -> u128 {
    let number = int_value(value);
    u128::try_from(number)
        .unwrap_or_else(|_| panic!("expected a non-negative integer, found {value}"))
}

fn bool_value(value: &Value) -> bool {
    value
        .as_bool()
        .unwrap_or_else(|| panic!("expected a boolean, found {value}"))
}

fn str_value(value: &Value) -> &str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("expected a string, found {value}"))
}

fn int_list(value: &Value) -> Vec<i128> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("expected an array, found {value}"))
        .iter()
        .map(int_value)
        .collect()
}

fn type_of(case: &Map<String, Value>) -> IntType {
    let name = str_value(field(case, "type"));
    IntType::from_name(name).unwrap_or_else(|| panic!("unknown type `{name}`"))
}

fn expected_res(value: &Value) -> Res {
    if let Some(name) = value.as_str() {
        Err(match name {
            "overflow" => Trap::Overflow,
            "div_zero" => Trap::DivZero,
            "range" => Trap::Range,
            other => panic!("unknown trap `{other}`"),
        })
    } else {
        Ok(int_value(value))
    }
}

fn check_res(function: &str, case: &Map<String, Value>, actual: Res) {
    let expected = expected_res(field(case, "result"));
    assert_eq!(
        actual, expected,
        "{function} case {case:?} disagrees with the exact model"
    );
}

fn bitwise_bool(function: &str, a: bool, b: bool) -> bool {
    match function {
        "AND" => a && b,
        "OR" => a || b,
        "XOR" => a ^ b,
        other => panic!("not a boolean bitwise function: {other}"),
    }
}

fn check_case(function: &str, case: &Map<String, Value>) {
    match function {
        "ADD" | "SUB" | "MUL" | "DIV" | "MOD" => {
            let ty = type_of(case);
            let a = int_value(field(case, "a"));
            let b = int_value(field(case, "b"));
            let result = match function {
                "ADD" => exact::add(ty, a, b),
                "SUB" => exact::sub(ty, a, b),
                "MUL" => exact::mul(ty, a, b),
                "DIV" => exact::div(ty, a, b),
                _ => exact::modulo(ty, a, b),
            };
            check_res(function, case, result);
        }
        "NEG" | "ABS" => {
            let ty = type_of(case);
            let a = int_value(field(case, "a"));
            let result = if function == "NEG" {
                exact::neg(ty, a)
            } else {
                exact::abs(ty, a)
            };
            check_res(function, case, result);
        }
        "AND" | "OR" | "XOR" => {
            let type_name = str_value(field(case, "type"));
            if type_name == "BOOL" {
                let a = bool_value(field(case, "a"));
                let b = bool_value(field(case, "b"));
                assert_eq!(
                    bool_value(field(case, "result")),
                    bitwise_bool(function, a, b),
                    "{function} case {case:?}"
                );
            } else {
                let ty = type_of(case);
                let a = nat_value(field(case, "a"));
                let b = nat_value(field(case, "b"));
                let result = match function {
                    "AND" => exact::bit_and(ty, a, b),
                    "OR" => exact::bit_or(ty, a, b),
                    _ => exact::bit_xor(ty, a, b),
                };
                assert_eq!(
                    nat_value(field(case, "result")),
                    result,
                    "{function} case {case:?}"
                );
            }
        }
        "NOT" => {
            let type_name = str_value(field(case, "type"));
            if type_name == "BOOL" {
                let a = bool_value(field(case, "a"));
                assert_eq!(bool_value(field(case, "result")), !a, "NOT case {case:?}");
            } else {
                let ty = type_of(case);
                let a = nat_value(field(case, "a"));
                assert_eq!(
                    nat_value(field(case, "result")),
                    exact::bit_not(ty, a),
                    "NOT case {case:?}"
                );
            }
        }
        "SHL" | "SHR" | "ROL" | "ROR" => {
            let ty = type_of(case);
            let a = nat_value(field(case, "a"));
            let n = i64::try_from(int_value(field(case, "n"))).expect("shift count");
            let result = match function {
                "SHL" => exact::shl(ty, a, n),
                "SHR" => exact::shr(ty, a, n),
                "ROL" => exact::rol(ty, a, n),
                _ => exact::ror(ty, a, n),
            };
            assert_eq!(
                nat_value(field(case, "result")),
                result,
                "{function} case {case:?}"
            );
        }
        "EQ" | "NE" | "LT" | "LE" | "GT" | "GE" => {
            let op = match function {
                "EQ" => Compare::Eq,
                "NE" => Compare::Ne,
                "LT" => Compare::Lt,
                "LE" => Compare::Le,
                "GT" => Compare::Gt,
                _ => Compare::Ge,
            };
            let a = int_value(field(case, "a"));
            let b = int_value(field(case, "b"));
            assert_eq!(
                bool_value(field(case, "result")),
                exact::compare(op, a, b),
                "{function} case {case:?}"
            );
        }
        "SEL" => {
            let g = bool_value(field(case, "g"));
            let a = int_value(field(case, "a"));
            let b = int_value(field(case, "b"));
            assert_eq!(
                int_value(field(case, "result")),
                exact::sel(g, a, b),
                "SEL case {case:?}"
            );
        }
        "MIN" | "MAX" => {
            let values = int_list(field(case, "values"));
            let result = if function == "MIN" {
                exact::min_list(&values)
            } else {
                exact::max_list(&values)
            }
            .expect("MIN/MAX vectors are non-empty");
            assert_eq!(
                int_value(field(case, "result")),
                result,
                "{function} case {case:?}"
            );
        }
        "LIMIT" => {
            let result = exact::limit(
                int_value(field(case, "mn")),
                int_value(field(case, "x")),
                int_value(field(case, "mx")),
            );
            assert_eq!(
                int_value(field(case, "result")),
                result,
                "LIMIT case {case:?}"
            );
        }
        "MUX" => {
            let values = int_list(field(case, "values"));
            let result = exact::mux(
                int_value(field(case, "k")),
                &values,
                int_value(field(case, "default")),
            );
            assert_eq!(
                int_value(field(case, "result")),
                result,
                "MUX case {case:?}"
            );
        }
        "CONV" => {
            let from = IntType::from_name(str_value(field(case, "from"))).expect("source type");
            let to = IntType::from_name(str_value(field(case, "to"))).expect("target type");
            check_res(
                function,
                case,
                exact::convert(from, to, int_value(field(case, "value"))),
            );
        }
        "LEN" | "LEN_CODE_UNIT" => {
            let kind = StringKind::from_name(str_value(field(case, "kind"))).expect("string kind");
            let text = str_value(field(case, "s"));
            if kind == StringKind::Sbcs {
                assert!(text.is_ascii(), "SBCS vector `{text}` is not ASCII");
            }
            let result = if function == "LEN" {
                exact::len(kind, text)
            } else {
                exact::len_code_unit(kind, text)
            };
            assert_eq!(
                usize::try_from(int_value(field(case, "result"))).expect("length"),
                result,
                "{function} case {case:?}"
            );
        }
        "LEN_MAX" => {
            let capacity = usize::try_from(int_value(field(case, "capacity"))).expect("capacity");
            assert_eq!(
                usize::try_from(int_value(field(case, "result"))).expect("length"),
                exact::len_max(capacity),
                "LEN_MAX case {case:?}"
            );
        }
        other => panic!("unknown exact function `{other}`"),
    }
}

fn render_value(value: &Value) -> String {
    match value {
        Value::Number(number) => number.to_string(),
        Value::String(text) => format!("\"{}\"", escape(text)),
        Value::Bool(flag) => flag.to_string(),
        Value::Array(items) => {
            let rendered: Vec<_> = items.iter().map(render_value).collect();
            format!("[{}]", rendered.join(","))
        }
        other => panic!("cannot canonicalize {other}"),
    }
}

fn escape(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other if (other as u32) < 0x20 => {
                use std::fmt::Write as _;
                let _ = write!(out, "\\u{:04x}", other as u32);
            }
            other => out.push(other),
        }
    }
    out
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn canonical(function: &str, cases: &[Value]) -> String {
    let mut text = format!("{function}\n");
    for value in cases {
        let case = object(value, "case");
        let fields: Vec<String> = case
            .iter()
            .map(|(key, value)| format!("{key}={}", render_value(value)))
            .collect();
        text.push_str(&fields.join(","));
        text.push('\n');
    }
    text
}

#[test]
fn every_exact_function_has_committed_matching_vectors() {
    let files = vector_files();
    let mut present = BTreeSet::new();
    for (file, value) in &files {
        let document = object(value, "vector");
        let function = str_value(field(document, "function")).to_owned();
        assert_eq!(
            document.get("version").map(int_value),
            Some(i128::from(exact::FORMAT_VERSION)),
            "{file}: unexpected format version"
        );
        assert_eq!(
            document.get("model_version").map(int_value),
            Some(i128::from(exact::MODEL_VERSION)),
            "{file}: unexpected model version"
        );
        let cases = document
            .get("cases")
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{file}: missing cases"));
        let generator = str_value(field(document, "generator"));
        let expected = format!("0x{:016x}", fnv1a(canonical(&function, cases).as_bytes()));
        assert_eq!(generator, expected, "{file}: generator digest drifted");
        for case in cases {
            check_case(&function, object(case, "case"));
        }
        present.insert((file.clone(), function));
    }

    let registered: BTreeSet<_> = exact::REGISTRY
        .iter()
        .map(|spec| (spec.file.to_owned(), spec.name.to_owned()))
        .collect();
    assert_eq!(
        present, registered,
        "vector files and the exact-function registry differ"
    );

    for spec in exact::REGISTRY {
        let value = files
            .iter()
            .find(|(file, _)| file == spec.file)
            .map(|(_, value)| value)
            .unwrap_or_else(|| panic!("missing vector file {}", spec.file));
        check_coverage(spec.name, spec.kind, value);
    }
}

fn check_coverage(function: &str, kind: FunctionKind, value: &Value) {
    let document = object(value, "vector");
    let cases = document
        .get("cases")
        .and_then(Value::as_array)
        .expect("cases");
    match kind {
        FunctionKind::Binary { types }
        | FunctionKind::Unary { types }
        | FunctionKind::Bitwise { types }
        | FunctionKind::Not { types }
        | FunctionKind::Shift { types }
        | FunctionKind::Compare { types }
        | FunctionKind::Select { types }
        | FunctionKind::MinMax { types }
        | FunctionKind::Limit { types }
        | FunctionKind::Mux { types } => {
            let seen: BTreeSet<_> = cases
                .iter()
                .map(|case| str_value(field(object(case, "case"), "type")).to_owned())
                .collect();
            for ty in types {
                assert!(
                    seen.contains(*ty),
                    "{function}: no vector case for type `{ty}`"
                );
            }
        }
        FunctionKind::Conv { types } => {
            let seen: BTreeSet<_> = cases
                .iter()
                .map(|case| {
                    let case = object(case, "case");
                    (
                        str_value(field(case, "from")).to_owned(),
                        str_value(field(case, "to")).to_owned(),
                    )
                })
                .collect();
            for from in types {
                for to in types {
                    assert!(
                        seen.contains(&((*from).to_owned(), (*to).to_owned())),
                        "{function}: no vector case for `{from}_TO_{to}`"
                    );
                }
            }
        }
        FunctionKind::Len { kinds } | FunctionKind::LenMax { kinds } => {
            let seen: BTreeSet<_> = cases
                .iter()
                .map(|case| str_value(field(object(case, "case"), "kind")).to_owned())
                .collect();
            for kind in kinds {
                assert!(
                    seen.contains(*kind),
                    "{function}: no vector case for string kind `{kind}`"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Constant-folding agreement with `stc-sema`.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum FoldOp {
    Binary(&'static str),
    Unary(&'static str),
    Conversion,
}

fn fold_op(function: &str) -> Option<FoldOp> {
    Some(match function {
        "ADD" => FoldOp::Binary("+"),
        "SUB" => FoldOp::Binary("-"),
        "MUL" => FoldOp::Binary("*"),
        "DIV" => FoldOp::Binary("/"),
        "MOD" => FoldOp::Binary("MOD"),
        "NEG" => FoldOp::Unary("-"),
        "AND" => FoldOp::Binary("AND"),
        "OR" => FoldOp::Binary("OR"),
        "XOR" => FoldOp::Binary("XOR"),
        "NOT" => FoldOp::Unary("NOT"),
        "EQ" => FoldOp::Binary("="),
        "NE" => FoldOp::Binary("<>"),
        "LT" => FoldOp::Binary("<"),
        "LE" => FoldOp::Binary("<="),
        "GT" => FoldOp::Binary(">"),
        "GE" => FoldOp::Binary(">="),
        "CONV" => FoldOp::Conversion,
        _ => return None,
    })
}

fn literal(ty: &str, value: i128) -> String {
    if ty == "BOOL" {
        return if value == 0 {
            "BOOL#FALSE".into()
        } else {
            "BOOL#TRUE".into()
        };
    }
    let layout = IntType::from_name(ty).expect("known type");
    if layout.signed {
        format!("{ty}#{value}")
    } else {
        format!("{ty}#16#{:X}", layout.to_bits(value))
    }
}

fn fold_expression(function: &str, case: &Map<String, Value>) -> String {
    match fold_op(function).expect("foldable function") {
        FoldOp::Binary(operator) => {
            let ty = str_value(field(case, "type"));
            if ty == "BOOL" {
                let a = bool_literal(field(case, "a"));
                let b = bool_literal(field(case, "b"));
                format!("{a} {operator} {b}")
            } else {
                let a = literal(ty, int_value(field(case, "a")));
                let b = literal(ty, int_value(field(case, "b")));
                format!("{a} {operator} {b}")
            }
        }
        FoldOp::Unary(operator) => {
            let ty = str_value(field(case, "type"));
            let a = if ty == "BOOL" {
                bool_literal(field(case, "a"))
            } else {
                literal(ty, int_value(field(case, "a")))
            };
            format!("{operator} {a}")
        }
        FoldOp::Conversion => {
            let from = str_value(field(case, "from"));
            let to = str_value(field(case, "to"));
            let value = literal(from, int_value(field(case, "value")));
            format!("{from}_TO_{to}({value})")
        }
    }
}

fn bool_literal(value: &Value) -> String {
    let flag = value.as_bool().unwrap_or_else(|| int_value(value) != 0);
    if flag {
        "BOOL#TRUE".into()
    } else {
        "BOOL#FALSE".into()
    }
}

fn result_type(function: &str, case: &Map<String, Value>) -> String {
    match function {
        "CONV" => str_value(field(case, "to")).to_owned(),
        "EQ" | "NE" | "LT" | "LE" | "GT" | "GE" => "BOOL".to_owned(),
        _ => str_value(field(case, "type")).to_owned(),
    }
}

#[derive(Debug)]
enum FoldOutcome {
    Value(stc_hir::ConstValue),
    Trap,
    Unsupported,
}

fn folded_value(source: &str) -> FoldOutcome {
    let (analysis, sink) = common::analyze_source(source);
    let failed = sink
        .iter()
        .any(|diagnostic| diagnostic.code == stc_sema::codes::CONST_FAILURE);
    if failed {
        return FoldOutcome::Trap;
    }
    if sink.has_errors() {
        // Some vector type pairs are not valid ST conversion functions; those
        // cases are not part of the folding agreement.
        return FoldOutcome::Unsupported;
    }
    let hir = &analysis.hir;
    let mut value = None;
    for def in &hir.defs {
        if let stc_hir::DefKind::Global {
            constant: true,
            init: Some(init),
            ..
        } = &def.kind
        {
            assert!(value.is_none(), "multiple constants in the folding source");
            let stc_hir::ExprKindHir::Literal(literal) = &hir.expr(*init).kind else {
                panic!("folded initializer is not a literal in:\n{source}");
            };
            value = Some(literal.value.clone());
        }
    }
    match value {
        Some(value) => FoldOutcome::Value(value),
        None => FoldOutcome::Unsupported,
    }
}

#[test]
fn constant_folding_agrees_with_exact_vectors() {
    let files = vector_files();
    let mut checked = 0usize;
    for (file, value) in &files {
        let document = object(value, "vector");
        let function = str_value(field(document, "function"));
        let Some(_) = fold_op(function) else {
            continue;
        };
        let cases = document
            .get("cases")
            .and_then(Value::as_array)
            .expect("cases");
        for case in cases {
            let case = object(case, "case");
            if function == "CONV" {
                let from = str_value(field(case, "from"));
                let to = str_value(field(case, "to"));
                if [from, to]
                    .iter()
                    .any(|ty| matches!(*ty, "CHAR" | "WCHAR" | "UCHAR"))
                {
                    continue;
                }
            }
            let ty = result_type(function, case);
            let expression = fold_expression(function, case);
            let source = format!(
                "VAR_GLOBAL CONSTANT\n  C : {ty} := {expression};\nEND_VAR\n\
                 PROGRAM P\nVAR\n  x : DINT;\nEND_VAR\n  x := 0;\nEND_PROGRAM\n"
            );
            let expected = field(case, "result");
            match folded_value(&source) {
                FoldOutcome::Unsupported => continue,
                FoldOutcome::Trap => assert!(
                    expected.is_string(),
                    "{file}: folding `{expression}` trapped but the model produced {expected:?}"
                ),
                FoldOutcome::Value(stc_hir::ConstValue::Int(value)) => {
                    assert!(
                        !expected.is_string(),
                        "{file}: folding `{expression}` to {value}, model expected {expected:?}"
                    );
                    assert_eq!(
                        value,
                        int_value(expected),
                        "{file}: folding `{expression}` disagrees with the exact model"
                    );
                }
                FoldOutcome::Value(stc_hir::ConstValue::Bool(value)) => {
                    let expected = if let Some(flag) = expected.as_bool() {
                        flag
                    } else {
                        int_value(expected) != 0
                    };
                    assert_eq!(
                        value, expected,
                        "{file}: folding `{expression}` disagrees with the exact model"
                    );
                }
                FoldOutcome::Value(other) => {
                    panic!("{file}: folding `{expression}` produced an unexpected value {other:?}")
                }
            }
            checked += 1;
        }
    }
    assert!(
        checked > 500,
        "constant-folding agreement sampled only {checked} cases"
    );
}
