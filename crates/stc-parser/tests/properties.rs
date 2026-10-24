mod common;

use stc_diag::DiagnosticSink;
use stc_lexer::{tokenize, tokenize_bytes};
use stc_parser::{ParseOptions, parse};
use stc_span::FileId;

const FILE: FileId = FileId::new(0);

fn lcg(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    *state
}

const TOKENS: &[&str] = &[
    ";",
    "(",
    ")",
    "[",
    "]",
    ",",
    ":",
    ".",
    ":=",
    "=>",
    "?=",
    "**",
    "..",
    "<>",
    "^",
    "+",
    "-",
    "*",
    "/",
    "=",
    "<",
    ">",
    "&",
    "%",
    "VAR",
    "END_VAR",
    "IF",
    "THEN",
    "ELSE",
    "END_IF",
    "CASE",
    "OF",
    "END_CASE",
    "FOR",
    "TO",
    "DO",
    "END_FOR",
    "FUNCTION",
    "END_FUNCTION",
    "END_PROGRAM",
    "PROGRAM",
    "TYPE",
    "END_TYPE",
    "STRUCT",
    "END_STRUCT",
    "REF",
    "NULL",
    "THIS",
    "{stc:test}",
    "x",
    "1",
    "T#1s",
    "%IX0.0",
    "U#'a'",
    "#if",
    "PROPERTY",
    "LD",
];

fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

fn mutate(source: &str, state: &mut u64) -> String {
    let mut out = source.to_string();
    let operations = (lcg(state) % 3) + 1;
    for _ in 0..operations {
        if out.is_empty() {
            out.push('x');
        }
        match lcg(state) % 4 {
            0 => {
                // Delete a character at a char boundary.
                let index = (lcg(state) as usize) % out.len();
                let start = floor_boundary(&out, index);
                let end = ceil_boundary(&out, index + 1);
                out.replace_range(start..end, "");
            }
            1 => {
                // Insert an arbitrary printable byte.
                let byte = b' ' + (lcg(state) % 95) as u8;
                let index = (lcg(state) as usize) % (out.len() + 1);
                let index = floor_boundary(&out, index);
                out.insert(index, byte as char);
            }
            2 => {
                // Remove a token-like run from the middle.
                let index = (lcg(state) as usize) % out.len();
                let start = floor_boundary(&out, index);
                let end = ceil_boundary(&out, index + 8);
                out.replace_range(start..end, "");
            }
            _ => {
                // Insert a grammar token.
                let token = TOKENS[(lcg(state) as usize) % TOKENS.len()];
                let index = (lcg(state) as usize) % (out.len() + 1);
                let index = floor_boundary(&out, index);
                out.insert_str(index, token);
            }
        }
    }
    out
}

const BASE: &str = r"
PROGRAM Main
VAR
  a, b : DINT;
  flag : BOOL;
  p : REF_TO DINT;
END_VAR
  a := 1 + 2 * 3;
  IF flag THEN
    a := a + 1;
  ELSE
    b := b - 1;
  END_IF;
  CASE a OF
    0 : b := 1;
    1..2 : b := 2;
    ELSE b := 3;
  END_CASE;
  p := REF(a);
  p^ := b;
END_PROGRAM
";

#[test]
fn mutated_sources_never_panic_and_stay_lossless() {
    let mut state = 0x5eed_1234_abcd_0001;
    for _ in 0..2000 {
        let source = mutate(BASE, &mut state);
        let mut sink = DiagnosticSink::with_limit(100);
        let stream = tokenize(FILE, &source, &mut sink);
        stream
            .check_invariants(&source)
            .unwrap_or_else(|message| panic!("{message}\n{source:?}"));
        let file = parse(
            FILE,
            source.as_str(),
            &stream,
            ParseOptions::default(),
            &mut sink,
        );
        file.tree().check_invariants().unwrap_or_else(|message| {
            panic!(
                "{message}\n{source:?}\n{}",
                stc_syntax::dump_cst(file.tree())
            )
        });
        assert_eq!(file.root().text(), source, "not lossless for {source:?}");
    }
}

#[test]
fn arbitrary_bytes_never_panic() {
    let mut state = 0x0123_4567_89ab_cdef;
    for _ in 0..1000 {
        let len = (lcg(&mut state) % 128) as usize;
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            // Bias toward ASCII punctuation so the lexer and parser see real tokens.
            let byte = if lcg(&mut state).is_multiple_of(3) {
                (lcg(&mut state) % 256) as u8
            } else {
                b' ' + (lcg(&mut state) % 95) as u8
            };
            bytes.push(byte);
        }
        let mut sink = DiagnosticSink::with_limit(100);
        let lexed = tokenize_bytes(FILE, &bytes, &mut sink);
        let file = parse(
            FILE,
            lexed.text.as_str(),
            &lexed.stream,
            ParseOptions::default(),
            &mut sink,
        );
        file.tree()
            .check_invariants()
            .unwrap_or_else(|message| panic!("{message}\nbytes: {bytes:?}"));
        assert_eq!(file.root().text(), lexed.text, "bytes: {bytes:?}");
    }
}

#[test]
fn relaxed_mode_is_a_superset_for_mutations() {
    let mut state = 0xfeed_face_0000_0002;
    for _ in 0..800 {
        let source = mutate(BASE, &mut state);
        let mut strict = DiagnosticSink::with_limit(100);
        let stream = tokenize(FILE, &source, &mut strict);
        let strict_file = parse(
            FILE,
            source.as_str(),
            &stream,
            ParseOptions::default(),
            &mut strict,
        );

        let mut relaxed = DiagnosticSink::with_limit(100);
        let stream = tokenize(FILE, &source, &mut relaxed);
        let relaxed_file = parse(
            FILE,
            source.as_str(),
            &stream,
            ParseOptions {
                relaxed_semicolons: true,
                ..ParseOptions::default()
            },
            &mut relaxed,
        );
        assert_eq!(relaxed_file.root().text(), source);
        assert_eq!(strict_file.root().text(), source);
    }
}
