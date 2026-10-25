//! Traceability from Grammar.md productions and rule IDs to named tests.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const PRODUCTIONS: &[(&str, &str, &str)] = &[
    (
        "source_file",
        "stc_gram_001_source_file_and_top_items",
        "stc_gram_001_source_file_negative",
    ),
    (
        "pragmas",
        "stc_gram_001_source_file_and_top_items",
        "stc_gram_001_source_file_negative",
    ),
    (
        "top_item",
        "stc_gram_001_source_file_and_top_items",
        "stc_gram_001_source_file_negative",
    ),
    (
        "qualified_name",
        "stc_gram_002_namespaces_and_using",
        "stc_gram_002_namespaces_and_using_negative",
    ),
    (
        "namespace_name",
        "stc_gram_002_namespaces_and_using",
        "stc_gram_002_namespaces_and_using_negative",
    ),
    (
        "using_directive",
        "stc_gram_002_namespaces_and_using",
        "stc_gram_002_namespaces_and_using_negative",
    ),
    (
        "namespace_decl",
        "stc_gram_002_namespaces_and_using",
        "stc_gram_002_namespaces_and_using_negative",
    ),
    (
        "namespace_element",
        "stc_gram_002_namespaces_and_using",
        "stc_gram_002_namespaces_and_using_negative",
    ),
    (
        "data_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "simple_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "subrange_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "enum_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "enum_value",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "named_value_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "named_value",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "array_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "struct_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "struct_elem",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "string_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "ref_type_decl",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "type_spec",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "array_type",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "array_bound",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "initializer",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "array_init",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "struct_init",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "struct_field_init",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "char_str",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "var_section",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "var_kind",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "var_qualifier",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "global_var_section",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "var_decl",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "var_type",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "edge_decl",
        "stc_gram_004_variable_declarations",
        "stc_gram_004_variable_declarations_negative",
    ),
    (
        "access_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "access_path",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "function_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "function_block_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "program_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "class_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "class_var_section",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "interface_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "method_decl",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "method_prototype",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "interface_list",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "access_spec",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "configuration_decl",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "resource_decl",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "single_resource_decl",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "task_config",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "task_attr",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "program_config",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "prog_conf_elem",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "fb_task",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "prog_cnxn",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "prog_data_source",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "data_sink",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "var_access_section",
        "stc_gram_005_pou_declarations",
        "stc_gram_005_pou_declarations_negative",
    ),
    (
        "config_init",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "config_inst_init",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "config_target",
        "stc_gram_006_configurations",
        "stc_gram_006_configurations_negative",
    ),
    (
        "stmt_list",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "assignment_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "ref_assignment",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "assignment_attempt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "call_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "return_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "exit_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "continue_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "if_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "case_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "case_selection",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "case_list",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "case_element",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "for_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "while_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "repeat_stmt",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "ref_name",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "ref_deref",
        "stc_gram_007_statements",
        "stc_gram_007_statements_negative",
    ),
    (
        "expression",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "or_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "xor_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "and_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "compare_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "equ_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "add_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "term",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "power_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "unary_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "postfix_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "postfix",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "primary_expr",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "variable_or_call",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "call_expression",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "path",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "arg_list",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "arg",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "ref_value",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "variable",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "literal",
        "stc_gram_008_expressions",
        "stc_gram_008_expressions_negative",
    ),
    (
        "place",
        "stc_gram_018_pointer_expressions",
        "stc_gram_018_pointer_expressions_negative",
    ),
    (
        "address_of",
        "stc_gram_018_pointer_expressions",
        "stc_gram_018_pointer_expressions_negative",
    ),
    (
        "sizeof_expr",
        "stc_gram_018_pointer_expressions",
        "stc_gram_018_pointer_expressions_negative",
    ),
    (
        "sizeof_operand",
        "stc_gram_018_pointer_expressions",
        "stc_gram_018_pointer_expressions_negative",
    ),
    (
        "pointer_conversion",
        "stc_gram_018_pointer_expressions",
        "stc_gram_018_pointer_expressions_negative",
    ),
    (
        "elementary_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "string_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "numeric_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "int_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "signed_int_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "unsigned_int_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "bit_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "time_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
    (
        "date_type_name",
        "stc_gram_003_type_declarations",
        "stc_gram_003_type_declarations_negative",
    ),
];

const RULES: &[(&str, &str)] = &[
    ("STC-GRAM-001", "stc_gram_001_source_file_and_top_items"),
    ("STC-GRAM-002", "stc_gram_002_namespaces_and_using"),
    ("STC-GRAM-003", "stc_gram_003_type_declarations"),
    ("STC-GRAM-004", "stc_gram_004_variable_declarations"),
    ("STC-GRAM-005", "stc_gram_005_pou_declarations"),
    ("STC-GRAM-006", "stc_gram_006_configurations"),
    ("STC-GRAM-007", "stc_gram_007_statements"),
    ("STC-GRAM-008", "stc_gram_008_expressions"),
    ("STC-GRAM-009", "stc_gram_009_comparison_precedence"),
    ("STC-GRAM-010", "stc_gram_010_power_associativity"),
    ("STC-GRAM-011", "stc_gram_011_unary_before_power"),
    ("STC-GRAM-012", "stc_gram_012_operator_aliases"),
    ("STC-GRAM-013", "stc_gram_013_short_circuit_syntax_parses"),
    ("STC-GRAM-014", "stc_gram_014_operand_order_parses"),
    ("STC-GRAM-015", "stc_gram_015_not_precedence"),
    ("STC-GRAM-016", "stc_gram_016_strict_and_relaxed_semicolons"),
    ("STC-GRAM-017", "stc_gram_017_pointer_types"),
    ("STC-GRAM-018", "stc_gram_018_pointer_expressions"),
];

const ERROR_CODES: &[(&str, &str)] = &[
    ("STC-E-GRAM-101", "stc_e_gram_101_il_body"),
    ("STC-E-GRAM-102", "stc_e_gram_102_sfc"),
    ("STC-E-GRAM-103", "stc_e_gram_103_preprocessor"),
    ("STC-E-GRAM-104", "stc_e_gram_104_ed4_constructs"),
    ("STC-E-GRAM-105", "stc_e_gram_105_unexpected_token"),
    ("STC-E-GRAM-106", "stc_e_gram_106_expected_token"),
    ("STC-E-GRAM-107", "stc_e_gram_107_missing_semicolon"),
    ("STC-E-GRAM-108", "stc_e_gram_108_mismatched_end"),
    ("STC-E-GRAM-109", "stc_e_gram_109_depth_limit"),
    ("STC-E-GRAM-110", "stc_e_gram_110_pragma_placement"),
    ("STC-E-GRAM-111", "stc_e_gram_111_missing_end"),
];

#[test]
fn every_grammar_production_has_positive_and_negative_tests() {
    let spec = std::fs::read_to_string(spec_path()).expect("read Grammar.md");
    let documented = productions(&spec);
    let mapped: BTreeSet<&str> = PRODUCTIONS.iter().map(|(name, _, _)| *name).collect();
    for production in &documented {
        assert!(
            mapped.contains(production.as_str()),
            "grammar production `{production}` is not in the traceability table"
        );
    }
    for (production, _, _) in PRODUCTIONS {
        assert!(
            documented.iter().any(|name| name == production),
            "traceability table lists `{production}`, which is not a Grammar §2–§10 production"
        );
    }
    let sources = crate_sources();
    for (production, positive, negative) in PRODUCTIONS {
        assert!(
            sources.contains(positive),
            "positive test `{positive}` for `{production}` is missing"
        );
        assert!(
            sources.contains(negative),
            "negative test `{negative}` for `{production}` is missing"
        );
    }
}

#[test]
fn rule_ids_have_named_tests() {
    assert_eq!(RULES.len(), 18);
    let sources = crate_sources();
    for (index, (rule, test)) in RULES.iter().enumerate() {
        assert_eq!(*rule, format!("STC-GRAM-{:03}", index + 1));
        assert!(sources.contains(rule), "rule {rule} is not referenced");
        assert!(sources.contains(test), "test {test} for {rule} is missing");
    }
}

#[test]
fn parser_error_codes_have_negative_tests() {
    assert_eq!(ERROR_CODES.len(), 11);
    let sources = crate_sources();
    for (index, (code, test)) in ERROR_CODES.iter().enumerate() {
        assert_eq!(*code, format!("STC-E-GRAM-{:03}", index + 101));
        assert!(sources.contains(code), "code {code} is not referenced");
        assert!(sources.contains(test), "test {test} for {code} is missing");
    }
}

fn productions(spec: &str) -> Vec<String> {
    let start = spec.find("## 2. Source unit").expect("grammar section 2");
    let end = spec
        .find("## 11. Disambiguation")
        .expect("grammar section 11");
    let mut names = Vec::new();
    for line in spec[start..end].lines() {
        let line = line.trim_start();
        let Some((name, _)) = line.split_once(" ::=") else {
            continue;
        };
        let name = name.trim();
        if name
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')
            && !names.iter().any(|existing| existing == name)
        {
            names.push(name.to_owned());
        }
    }
    names
}

fn crate_sources() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = String::new();
    for directory in ["src", "tests"] {
        collect_rust_sources(&root.join(directory), &mut sources);
    }
    sources
}

fn collect_rust_sources(directory: &Path, output: &mut String) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            output.push_str(&std::fs::read_to_string(path).unwrap_or_default());
        }
    }
}

fn spec_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Specification/Syntax/Grammar.md")
}
