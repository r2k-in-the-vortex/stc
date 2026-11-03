//! Differential certificate for the shared scalar kernel paths.
//!
//! Phase 2 (Testing and Verification §5.2) requires the three Rust
//! implementations of the scalar rules to agree on the shared subset:
//!
//! - the reference model in [`stc_codegen::exact`],
//! - the analyzer's constant folder (`crates/stc-sema/src/consteval.rs`),
//! - codegen's `cast_value`/`int_convert` (`crates/stc-codegen/src/expr.rs`).
//!
//! The vector tests already compare the reference model and the constant
//! folder on generated cases. This file adds the conversion certificate:
//! every conversion accepted by the front end is folded and compared with
//! `exact::convert`, and the emitted IR shape for a runtime conversion is
//! checked against the same widening/narrowing/reinterpretation rules.
//!
//! These are tests, not a refinement proof. Production unification of the
//! three paths remains an owner decision; until then the agreement is V0
//! evidence for the shared scalar subset.

mod common;

use stc_codegen::Mode;
use stc_codegen::exact::{self, IntType, Trap};

/// One folded-constant outcome.
#[derive(Debug)]
enum Folded {
    /// The folder produced a literal value.
    Value(stc_hir::ConstValue),
    /// The folder reported a constant failure (the model's `Trap`).
    Trap,
    /// The front end rejects the conversion or the case is outside the
    /// folding subset.
    Unsupported,
}

/// Formats a typed literal accepted by the front end.
fn literal(ty: &str, value: i128) -> String {
    if ty == "BOOL" {
        return if value == 0 {
            "BOOL#FALSE".into()
        } else {
            "BOOL#TRUE".into()
        };
    }
    let layout = IntType::from_name(ty).expect("known conversion type");
    if layout.signed {
        format!("{ty}#{value}")
    } else {
        format!("{ty}#16#{:X}", layout.to_bits(value))
    }
}

/// Boundary values representable in `ty`.
fn sample_values(ty: &str) -> Vec<i128> {
    let layout = IntType::from_name(ty).expect("known conversion type");
    if ty == "BOOL" {
        return vec![0, 1];
    }
    let mut values = vec![
        layout.minimum(),
        layout.minimum() + 1,
        0,
        1,
        layout.maximum() - 1,
        layout.maximum(),
    ];
    if layout.signed {
        values.push(-1);
    }
    values.sort_unstable();
    values.dedup();
    values
}

/// Folds a single constant global initializer.
fn folded_value(source: &str) -> Folded {
    let (analysis, sink) = common::analyze_source(source);
    let has_code = |code: &str| sink.iter().any(|diagnostic| diagnostic.code == code);
    if has_code(stc_sema::codes::CONST_FAILURE) {
        return Folded::Trap;
    }
    if has_code(stc_sema::codes::CONV_NONE) {
        return Folded::Unsupported;
    }
    assert!(
        !sink.has_errors(),
        "unexpected errors for:\n{source}\n{:#?}",
        sink.diagnostics()
    );
    let hir = &analysis.hir;
    let mut value = None;
    for def in &hir.defs {
        if let stc_hir::DefKind::Global {
            constant: true,
            init: Some(init),
            ..
        } = &def.kind
        {
            let stc_hir::ExprKindHir::Literal(literal) = &hir.expr(*init).kind else {
                panic!("folded initializer is not a literal in:\n{source}");
            };
            value = Some(literal.value.clone());
        }
    }
    Folded::Value(value.unwrap_or_else(|| panic!("no folded constant in:\n{source}")))
}

const PROGRAM_SHELL: &str = "PROGRAM P\nVAR\n  x : DINT;\nEND_VAR\n  x := 0;\nEND_PROGRAM\n";

/// Every conversion pair the front end accepts folds to `exact::convert`.
#[test]
fn constant_conversions_match_the_exact_reference() {
    let mut checked_pairs = 0usize;
    let mut checked_cases = 0usize;
    for from in exact::CONV_TYPES {
        for to in exact::CONV_TYPES {
            let source_layout = IntType::from_name(from).expect("known source type");
            let target_layout = IntType::from_name(to).expect("known target type");
            let mut pair_cases = 0usize;
            for value in sample_values(from) {
                let expression = format!("{from}_TO_{to}({})", literal(from, value));
                let source = format!(
                    "VAR_GLOBAL CONSTANT\n  C : {to} := {expression};\nEND_VAR\n\
                     {PROGRAM_SHELL}"
                );
                let expected = exact::convert(source_layout, target_layout, value);
                match folded_value(&source) {
                    Folded::Unsupported => break,
                    Folded::Trap => {
                        assert!(
                            matches!(expected, Err(Trap::Range)),
                            "folding `{expression}` trapped but the exact model returned {expected:?}"
                        );
                    }
                    Folded::Value(stc_hir::ConstValue::Int(folded)) => {
                        assert_eq!(
                            expected,
                            Ok(folded),
                            "folding `{expression}` disagrees with the exact model"
                        );
                    }
                    Folded::Value(stc_hir::ConstValue::Bool(folded)) => {
                        assert_eq!(
                            expected,
                            Ok(i128::from(folded)),
                            "folding `{expression}` disagrees with the exact model"
                        );
                    }
                    Folded::Value(stc_hir::ConstValue::Char(folded)) => {
                        assert_eq!(
                            expected,
                            Ok(i128::from(folded)),
                            "folding `{expression}` disagrees with the exact model"
                        );
                    }
                    Folded::Value(other) => {
                        panic!("folding `{expression}` produced unexpected value {other:?}");
                    }
                }
                pair_cases += 1;
                checked_cases += 1;
            }
            if pair_cases > 0 {
                checked_pairs += 1;
            }
        }
    }
    assert!(
        checked_pairs >= 170,
        "conversion certificate sampled only {checked_pairs} type pairs"
    );
    assert!(
        checked_cases >= 800,
        "conversion certificate sampled only {checked_cases} cases"
    );
}

/// A reference model for one binary arithmetic operator.
type BinaryModel = fn(IntType, i128, i128) -> exact::Res;

const ARITHMETIC_OPERATORS: &[(&str, BinaryModel)] = &[
    ("+", exact::add),
    ("-", exact::sub),
    ("*", exact::mul),
    ("/", exact::div),
    ("MOD", exact::modulo),
];

const COMPARISON_OPERATORS: &[(&str, exact::Compare)] = &[
    ("=", exact::Compare::Eq),
    ("<>", exact::Compare::Ne),
    ("<", exact::Compare::Lt),
    ("<=", exact::Compare::Le),
    (">", exact::Compare::Gt),
    (">=", exact::Compare::Ge),
];

fn constant_source(ty: &str, expression: &str) -> String {
    format!("VAR_GLOBAL CONSTANT\n  C : {ty} := {expression};\nEND_VAR\n{PROGRAM_SHELL}")
}

/// Asserts that folding `expression` agrees with an exact-model result.
fn assert_folds(result_ty: &str, expression: &str, expected: exact::Res) {
    let source = constant_source(result_ty, expression);
    match folded_value(&source) {
        Folded::Trap => assert!(
            expected.is_err(),
            "folding `{expression}` trapped but the exact model returned {expected:?}"
        ),
        Folded::Value(stc_hir::ConstValue::Int(folded)) => assert_eq!(
            expected,
            Ok(folded),
            "folding `{expression}` disagrees with the exact model"
        ),
        Folded::Value(stc_hir::ConstValue::Bool(folded)) => assert_eq!(
            expected,
            Ok(i128::from(folded)),
            "folding `{expression}` disagrees with the exact model"
        ),
        Folded::Value(other) => {
            panic!("folding `{expression}` produced unexpected value {other:?}");
        }
        Folded::Unsupported => {
            panic!("front end rejected foldable expression `{expression}`");
        }
    }
}

/// Integer arithmetic, bitwise operations, and comparisons fold exactly as
/// the reference model computes them on boundary values.
///
/// Shifts, rotates, `ABS`, and the selection functions have no constant
/// folding syntax in the analyzer, so they are outside this differential
/// subset; the vector tests still cover the reference model itself.
#[test]
fn constant_arithmetic_matches_the_exact_reference() {
    for ty_name in exact::INT_TYPES {
        let ty = IntType::from_name(ty_name).expect("known integer type");
        for a in sample_values(ty_name) {
            let a_literal = literal(ty_name, a);
            let expected = exact::neg(ty, a);
            assert_folds(ty_name, &format!("-{a_literal}"), expected);
            for b in sample_values(ty_name) {
                let b_literal = literal(ty_name, b);
                for (operator, model) in ARITHMETIC_OPERATORS {
                    let expression = format!("{a_literal} {operator} {b_literal}");
                    assert_folds(ty_name, &expression, model(ty, a, b));
                }
                for (operator, comparison) in COMPARISON_OPERATORS {
                    let expression = format!("{a_literal} {operator} {b_literal}");
                    let expected = Ok(i128::from(exact::compare(*comparison, a, b)));
                    assert_folds("BOOL", &expression, expected);
                }
            }
        }
    }

    for ty_name in exact::BIT_TYPES {
        let ty = IntType::from_name(ty_name).expect("known bit-string type");
        for a in sample_values(ty_name) {
            let a_literal = literal(ty_name, a);
            for b in sample_values(ty_name) {
                let b_literal = literal(ty_name, b);
                for (operator, model) in [
                    ("AND", exact::bit_and as fn(IntType, u128, u128) -> u128),
                    ("OR", exact::bit_or),
                    ("XOR", exact::bit_xor),
                ] {
                    let expression = format!("{a_literal} {operator} {b_literal}");
                    let expected = i128::try_from(model(ty, a as u128, b as u128))
                        .expect("bit-string result fits i128");
                    assert_folds(ty_name, &expression, Ok(expected));
                }
                let expression = format!("NOT {a_literal}");
                let expected = i128::try_from(exact::bit_not(ty, a as u128))
                    .expect("bit-string result fits i128");
                assert_folds(ty_name, &expression, Ok(expected));
            }
        }
    }

    for a in [false, true] {
        let a_literal = literal("BOOL", i128::from(a));
        assert_folds("BOOL", &format!("NOT {a_literal}"), Ok(i128::from(!a)));
        for b in [false, true] {
            let b_literal = literal("BOOL", i128::from(b));
            assert_folds(
                "BOOL",
                &format!("{a_literal} AND {b_literal}"),
                Ok(i128::from(a && b)),
            );
            assert_folds(
                "BOOL",
                &format!("{a_literal} OR {b_literal}"),
                Ok(i128::from(a || b)),
            );
            assert_folds(
                "BOOL",
                &format!("{a_literal} XOR {b_literal}"),
                Ok(i128::from(a ^ b)),
            );
        }
    }
}

/// Extracts the IR of one program body.
fn program_body<'a>(module: &'a str, program: &str) -> &'a str {
    let marker = format!("define internal void @stc_prog__ROOT_n_{program}_body(");
    let start = module
        .find(&marker)
        .unwrap_or_else(|| panic!("no `{program}` body in module:\n{module}"));
    let rest = &module[start..];
    let end = rest.find("\ndefine ").unwrap_or(rest.len());
    &rest[..end]
}

/// Generates one program with a runtime conversion and returns its body.
fn conversion_body(from: &str, to: &str) -> String {
    let source = format!(
        "PROGRAM P\nVAR\n  a : {from};\n  b : {to};\nEND_VAR\n  b := {from}_TO_{to}(a);\n\
         END_PROGRAM\nCONFIGURATION C\nRESOURCE R ON Cpu\n  PROGRAM inst : P;\n\
         END_RESOURCE\nEND_CONFIGURATION\n"
    );
    let output = common::generate(&source, Mode::Executable);
    program_body(&output.module, "P").to_owned()
}

/// Codegen emits widening as an extension, narrowing as a range check plus
/// truncation, equal widths as a reinterpretation, and `BOOL` targets as a
/// zero test, matching `exact::convert`.
#[test]
fn runtime_conversions_emit_the_exact_model_shape() {
    let widening_signed = conversion_body("SINT", "DINT");
    assert!(
        widening_signed.contains("sext") && !widening_signed.contains("trunc"),
        "{widening_signed}"
    );
    assert!(
        !widening_signed.contains("unreachable"),
        "{widening_signed}"
    );

    let widening_unsigned = conversion_body("USINT", "DINT");
    assert!(
        widening_unsigned.contains("zext") && !widening_unsigned.contains("trunc"),
        "{widening_unsigned}"
    );

    let narrowing = conversion_body("DINT", "SINT");
    assert!(
        narrowing.contains("trunc") && narrowing.contains("unreachable"),
        "{narrowing}"
    );

    let reinterpret = conversion_body("DINT", "UDINT");
    assert!(
        !reinterpret.contains("sext")
            && !reinterpret.contains("zext")
            && !reinterpret.contains("trunc")
            && !reinterpret.contains("unreachable"),
        "{reinterpret}"
    );

    let unsigned_reinterpret = conversion_body("UDINT", "DINT");
    assert!(
        !unsigned_reinterpret.contains("sext")
            && !unsigned_reinterpret.contains("zext")
            && !unsigned_reinterpret.contains("trunc")
            && !unsigned_reinterpret.contains("unreachable"),
        "{unsigned_reinterpret}"
    );

    let to_bool = conversion_body("DINT", "BOOL");
    assert!(to_bool.contains("icmp ne"), "{to_bool}");

    let from_bool = conversion_body("BOOL", "DINT");
    assert!(from_bool.contains("zext"), "{from_bool}");
}
