//! `stc-fmt` unit tests: fixed style, comments/trivia, relaxed semicolons,
//! indentation, encoding, error handling, and idempotence.

use stc_diag::DiagnosticSink;
use stc_fmt::{SYNTAX_ERROR, format_bytes};
use stc_span::FileId;

/// Formats one clean file and checks that the result is idempotent.
fn format(source: &str) -> String {
    format_bytes_checked(source.as_bytes())
}

fn format_bytes_checked(bytes: &[u8]) -> String {
    let mut diagnostics = DiagnosticSink::new();
    let outcome = format_bytes(FileId::new(0), bytes, &mut diagnostics);
    assert!(
        !diagnostics.has_errors(),
        "unexpected diagnostics: {:?}",
        diagnostics.diagnostics()
    );
    let formatted = outcome.formatted.expect("a clean file is formatted");
    let mut second = DiagnosticSink::new();
    let again = format_bytes(FileId::new(0), formatted.as_bytes(), &mut second);
    assert!(
        !second.has_errors(),
        "formatted output did not parse: {:?}",
        second.diagnostics()
    );
    assert_eq!(
        formatted,
        again.formatted.expect("second pass formats"),
        "formatter is not idempotent"
    );
    formatted
}

#[test]
fn relaxed_semicolons_become_strict() {
    let source = "\
program main
var
counter:dint;end_var
if counter>0 then counter:=counter-1;else counter:=add(counter,1);end_if
end_program
";
    let expected = "\
PROGRAM main
VAR
  counter : DINT;
END_VAR
  IF counter > 0 THEN
    counter := counter - 1;
  ELSE
    counter := add(counter, 1);
  END_IF;
END_PROGRAM
";
    assert_eq!(format(source), expected);
}

#[test]
fn spacing_is_normalized() {
    let source = "\
FUNCTION F : DINT
VAR
  a,b:DINT;
  p:REF_TO DINT;
  arr:ARRAY [1..3] OF DINT;
  t:STRING[32];
END_VAR
  a:=b+1;
  p:=REF(a);
  arr[0]:=(a+1)*2**2;
  F:=SIZEOF(DINT);
  IF a<>b AND NOT TRUE THEN a:=a MOD 2; END_IF
END_FUNCTION
";
    let expected = "\
FUNCTION F : DINT
VAR
  a, b : DINT;
  p : REF_TO DINT;
  arr : ARRAY [1..3] OF DINT;
  t : STRING[32];
END_VAR
  a := b + 1;
  p := REF(a);
  arr[0] := (a + 1) * 2 ** 2;
  F := SIZEOF(DINT);
  IF a <> b AND NOT TRUE THEN
    a := a MOD 2;
  END_IF;
END_FUNCTION
";
    assert_eq!(format(source), expected);
}

#[test]
fn postfix_deref_keeps_operator_spacing() {
    let source = "\
PROGRAM p
VAR
  r : REF_TO DINT;
  a : DINT;
END_VAR
  r^:=a;
  IF r^ THEN a:=1; END_IF
END_PROGRAM
";
    let expected = "\
PROGRAM p
VAR
  r : REF_TO DINT;
  a : DINT;
END_VAR
  r^ := a;
  IF r^ THEN
    a := 1;
  END_IF;
END_PROGRAM
";
    assert_eq!(format(source), expected);
}

#[test]
fn comments_and_blank_lines_are_preserved() {
    let source = "\
(* header *)

FUNCTION F : DINT
VAR // variables
  a : DINT; (* note *)

END_VAR
  // leading
  a := 1; /* tail */
END_FUNCTION
";
    assert_eq!(format(source), source);
}

#[test]
fn indentation_nests_blocks() {
    let source = "\
program main
var
x:INT;
end_var
if x>0 then
x:=x-1;
while x>0 do
x:=x-1;
end_while
else
x:=0;
end_if
end_program
";
    let expected = "\
PROGRAM main
VAR
  x : INT;
END_VAR
  IF x > 0 THEN
    x := x - 1;
    WHILE x > 0 DO
      x := x - 1;
    END_WHILE;
  ELSE
    x := 0;
  END_IF;
END_PROGRAM
";
    assert_eq!(format(source), expected);
}

#[test]
fn declarations_are_one_per_line() {
    let source = "\
TYPE Speed : INT (0..100); Point : STRUCT x : REAL; y : REAL; END_STRUCT; END_TYPE
CONFIGURATION Plant
RESOURCE Cpu ON Cpu
TASK Fast(SINGLE := T#10ms); PROGRAM main_instance WITH Fast : MainProgram;
END_RESOURCE
END_CONFIGURATION
";
    let expected = "\
TYPE
  Speed : INT (0..100);
  Point : STRUCT
    x : REAL;
    y : REAL;
  END_STRUCT;
END_TYPE
CONFIGURATION Plant
RESOURCE Cpu ON Cpu
  TASK Fast(SINGLE := T#10ms);
  PROGRAM main_instance WITH Fast : MainProgram;
END_RESOURCE
END_CONFIGURATION
";
    assert_eq!(format(source), expected);
}

#[test]
fn crlf_and_bom_are_preserved() {
    let source = "\u{feff}program p\r\nvar\r\nx:int;\r\nend_var\r\nx:=1;\r\nif x>0 then x:=2;end_if\r\nend_program\r\n";
    let expected = "\u{feff}PROGRAM p\r\nVAR\r\n  x : INT;\r\nEND_VAR\r\n  x := 1;\r\n  IF x > 0 THEN\r\n    x := 2;\r\n  END_IF;\r\nEND_PROGRAM\r\n";
    let formatted = format_bytes_checked(source.as_bytes());
    assert_eq!(formatted, expected);
    assert_eq!(
        format_bytes_checked(formatted.as_bytes()),
        expected,
        "CRLF output is not idempotent"
    );
}

#[test]
fn existing_line_endings_keep_their_encoding() {
    // CRLF is dominant, but the line ending inside the function body is LF and
    // must not be rewritten.
    let source = "PROGRAM p\r\nVAR\n  x : INT;\r\nEND_VAR\r\n  x := 1;\r\nEND_PROGRAM\r\n";
    let expected = "PROGRAM p\r\nVAR\n  x : INT;\r\nEND_VAR\r\n  x := 1;\r\nEND_PROGRAM\r\n";
    assert_eq!(format(source), expected);
}

#[test]
fn unknown_tokens_keep_their_spelling() {
    let source = "\
{stc:test}
FUNCTION SmokeTest : BOOL
VAR
  d : DINT := 16#AbCd;
  s : STRING := 'AbC';
  u : DINT := DINT#-5;
  w AT %MW10 : DINT;
END_VAR
  SmokeTest := d = 16#AbCd;
END_FUNCTION
";
    assert_eq!(format(source), source);
}

#[test]
fn syntax_errors_prevent_formatting() {
    let mut diagnostics = DiagnosticSink::new();
    let outcome = format_bytes(FileId::new(0), b"PROGRAM p\n  x := 1;\n", &mut diagnostics);
    assert!(outcome.formatted.is_none());
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == SYNTAX_ERROR),
        "missing {SYNTAX_ERROR}: {:?}",
        diagnostics.diagnostics()
    );
    assert!(diagnostics.has_errors());
}

#[test]
fn empty_and_comment_only_files_are_stable() {
    assert_eq!(format(""), "");
    assert_eq!(format("// only a comment\n"), "// only a comment\n");
    assert_eq!(format("(* a *)\n\n(* b *)\n"), "(* a *)\n\n(* b *)\n");
    assert_eq!(
        format_bytes_checked("\u{feff}".as_bytes()),
        "\u{feff}",
        "a BOM-only file is preserved"
    );
}
