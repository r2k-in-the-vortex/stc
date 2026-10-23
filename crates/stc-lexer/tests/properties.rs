mod common;

use common::{FILE, lex, lex_bytes};
use stc_diag::DiagnosticSink;
use stc_lexer::tokenize;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

#[test]
fn arbitrary_bytes_never_panic_and_preserve_coverage() {
    let mut rng = Rng::new(0x5eed_1234_5678_9abc);
    for _ in 0..2500 {
        let length = rng.below(256);
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            bytes.push((rng.next() & 0xFF) as u8);
        }
        let (lexed, _) = lex_bytes(&bytes);
        assert_eq!(lexed.stream.reconstruct(&lexed.text), lexed.text);
        assert!(lexed.stream.eof().is_some());
    }
}

#[test]
fn arbitrary_valid_utf8_never_panics_and_preserves_coverage() {
    const ALPHABET: &[char] = &[
        'a', 'Z', '_', '0', '9', ' ', '\t', '\n', '\r', '\u{0b}', '\u{0c}', '\'', '"', '$', '{',
        '}', '(', ')', '*', '/', '%', ':', ';', ',', '.', '#', '^', '+', '-', '<', '>', '=', '&',
        'é', '日', '😀', '\u{feff}',
    ];
    let mut rng = Rng::new(0x1234_5678_9abc_def0);
    for _ in 0..2500 {
        let length = rng.below(160);
        let mut source = String::new();
        for _ in 0..length {
            source.push(ALPHABET[rng.below(ALPHABET.len())]);
        }
        let (stream, _) = lex(&source);
        assert_eq!(stream.reconstruct(&source), source);
    }
}

#[test]
fn mixed_line_endings_never_break_coverage() {
    let mut rng = Rng::new(0x0bad_f00d_dead_beef);
    for _ in 0..1000 {
        let mut source = String::new();
        for _ in 0..rng.below(40) {
            match rng.below(4) {
                0 => source.push('\n'),
                1 => source.push_str("\r\n"),
                2 => source.push('\r'),
                _ => source.push_str(" // x"),
            }
            source.push('a');
        }
        let (stream, _) = lex(&source);
        assert_eq!(stream.reconstruct(&source), source);
    }
}

#[test]
fn every_ascii_character_is_total() {
    for value in 0u8..=127 {
        let source = char::from(value).to_string();
        let (stream, _) = lex(&source);
        assert_eq!(stream.reconstruct(&source), source);
        let doubled = format!("{source}{source}");
        let (stream, _) = lex(&doubled);
        assert_eq!(stream.reconstruct(&doubled), doubled);
    }
}

#[test]
fn lexing_is_deterministic() {
    let source =
        "PROGRAM Main\nVAR x : INT := 16#FF; END_VAR\n// note\nU#'héllo ${1F600}'\nEND_PROGRAM\n";
    let mut first = DiagnosticSink::unlimited();
    let mut second = DiagnosticSink::unlimited();
    let a = tokenize(FILE, source, &mut first);
    let b = tokenize(FILE, source, &mut second);
    assert_eq!(a, b);
    assert_eq!(first.diagnostics(), second.diagnostics());
}
