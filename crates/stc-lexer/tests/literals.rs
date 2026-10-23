mod common;

use common::*;
use stc_lexer::{
    CharKind, DurationComponent, ElementaryType, Literal, NumericKind, NumericLiteral, Punct,
    Radix, Sign, StringEncoding, TimeUnit, TokenKind, TypedLiteral, TypedValue,
};

fn numeric(source: &str) -> NumericLiteral {
    let token = first_token(source);
    token
        .numeric()
        .cloned()
        .unwrap_or_else(|| panic!("expected a numeric literal for {source:?}"))
}

fn typed(source: &str) -> TypedLiteral {
    let token = first_token(source);
    let Some(Literal::Typed(typed)) = token.literal.as_deref() else {
        panic!("expected a typed literal for {source:?}");
    };
    typed.clone()
}

fn literal(source: &str) -> Literal {
    let token = first_token(source);
    *token
        .literal
        .unwrap_or_else(|| panic!("expected a literal for {source:?}"))
}

fn string_units(source: &str) -> Vec<u32> {
    let token = first_token(source);
    let Some(Literal::String(string)) = token.literal.as_deref() else {
        panic!("expected a string literal for {source:?}");
    };
    string.units.clone()
}

#[test]
fn stc_lex_017_decimal_integer_literals() {
    let stream = lex_clean("0 123_4 0012");
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens.len(), 3);
    assert_eq!(numeric("123_4").text.as_ref(), "1234");
    assert_eq!(numeric("0012").text.as_ref(), "0012");
    assert_eq!(numeric("0").radix, Radix::Decimal);
    assert_eq!(numeric("0").kind, NumericKind::Integer);

    assert_code("1__2", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("1_", stc_lexer::codes::MALFORMED_LITERAL);
    assert_eq!(first_token("_1").kind, TokenKind::Ident("_1".into()));
}

#[test]
fn stc_lex_018_real_literals() {
    assert_eq!(numeric("0.0").text.as_ref(), "0.0");
    assert_eq!(numeric("3.14159_26").text.as_ref(), "3.1415926");
    assert_eq!(numeric("1.34E-12").text.as_ref(), "1.34E-12");
    assert_eq!(numeric("1_0.0_1e+2_0").text.as_ref(), "10.01E+20");
    assert_eq!(numeric("1.5").kind, NumericKind::Real);

    assert_code("1.0E", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("1.0E+", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("1E3", stc_lexer::codes::MALFORMED_LITERAL);
    assert_eq!(
        clean_kinds("1."),
        vec![TokenKind::IntLiteral, TokenKind::Punct(Punct::Dot)]
    );
    assert_eq!(
        clean_kinds("1..2"),
        vec![
            TokenKind::IntLiteral,
            TokenKind::Punct(Punct::Range),
            TokenKind::IntLiteral,
        ]
    );
}

#[test]
fn stc_lex_020_based_literals() {
    let cases = [
        ("2#1010", Radix::Binary, "1010"),
        ("8#377", Radix::Octal, "377"),
        ("16#DEAD_BEEF", Radix::Hexadecimal, "DEADBEEF"),
        ("16#dead_beef", Radix::Hexadecimal, "DEADBEEF"),
    ];
    for (source, radix, digits) in cases {
        let numeric = numeric(source);
        assert_eq!(numeric.radix, radix, "{source}");
        assert_eq!(numeric.kind, NumericKind::Integer);
        assert_eq!(numeric.text.as_ref(), digits);
    }

    for invalid in ["2#102", "16#G", "16#_1", "16#1_", "2#"] {
        assert_code(invalid, stc_lexer::codes::MALFORMED_LITERAL);
    }
}

#[test]
fn stc_lex_021_octal_literals_warn() {
    let (stream, sink) = lex("8#377");
    assert_eq!(numeric("8#377").radix, Radix::Octal);
    assert_eq!(tokens_without_eof(&stream).len(), 1);
    assert!(has_code(&sink, stc_lexer::codes::DEPRECATED_OCTAL));
}

#[test]
fn stc_lex_022_typed_numeric_literals() {
    let int = typed("INT#-123");
    assert_eq!(int.ty, ElementaryType::Int);
    assert_eq!(int.sign, Some(Sign::Minus));
    let TypedValue::Numeric(value) = &int.value else {
        panic!("expected numeric value");
    };
    assert_eq!(value.text.as_ref(), "123");

    let based = typed("INT#16#7FFF");
    let TypedValue::Numeric(value) = &based.value else {
        panic!("expected numeric value");
    };
    assert_eq!(value.radix, Radix::Hexadecimal);
    assert_eq!(value.text.as_ref(), "7FFF");

    let real = typed("REAL#1.0e3");
    let TypedValue::Numeric(value) = &real.value else {
        panic!("expected numeric value");
    };
    assert_eq!(value.kind, NumericKind::Real);
    assert_eq!(value.text.as_ref(), "1.0E3");

    let boolean = typed("BOOL#FALSE");
    assert_eq!(boolean.ty, ElementaryType::Bool);
    assert_eq!(boolean.value, TypedValue::Bool(false));

    let character = typed("CHAR#16#41");
    assert_eq!(character.ty, ElementaryType::Char);
    let TypedValue::Numeric(value) = &character.value else {
        panic!("expected numeric value");
    };
    assert_eq!(value.text.as_ref(), "41");

    assert_code("INT#", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("INT#x", stc_lexer::codes::MALFORMED_LITERAL);
}

#[test]
fn stc_lex_023_raw_digits_survive_for_the_analyzer() {
    let digits = "999999999999999999999999999999999999999999";
    let source = format!("DINT#{digits}");
    assert_eq!(
        typed(&source).value,
        TypedValue::Numeric(NumericLiteral {
            kind: NumericKind::Integer,
            radix: Radix::Decimal,
            text: digits.into(),
        })
    );
}

#[test]
fn stc_lex_024_boolean_literals() {
    assert_eq!(
        clean_kinds("TRUE FALSE"),
        vec![
            TokenKind::Keyword(stc_lexer::Keyword::True),
            TokenKind::Keyword(stc_lexer::Keyword::False),
        ]
    );
    assert_eq!(typed("BOOL#TRUE").value, TypedValue::Bool(true));
    assert_eq!(typed("BOOL#FALSE").value, TypedValue::Bool(false));
    assert_eq!(
        typed("BOOL#0").value,
        TypedValue::Numeric(NumericLiteral {
            kind: NumericKind::Integer,
            radix: Radix::Decimal,
            text: "0".into(),
        })
    );
    assert_eq!(
        typed("BOOL#1").value,
        TypedValue::Numeric(NumericLiteral {
            kind: NumericKind::Integer,
            radix: Radix::Decimal,
            text: "1".into(),
        })
    );
}

#[test]
fn stc_lex_026_raw_string_character_restrictions() {
    assert_eq!(string_units("'a b'"), vec![0x61, 0x20, 0x62]);
    assert_code("'é'", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("'a\tb'", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("\"a$zb\"", stc_lexer::codes::INVALID_ESCAPE);
    assert_code("\"a$b\"", stc_lexer::codes::INVALID_ESCAPE);

    assert_eq!(string_units("\"é😀\""), vec![0xE9, 0xD83D, 0xDE00]);
}

#[test]
fn stc_lex_027_escape_sequences() {
    let source = r#"'$$ $' $" $L $N $P $R $T $41'"#;
    assert_eq!(
        string_units(source),
        vec![
            0x24, 0x20, 0x27, 0x20, 0x22, 0x20, 0x0A, 0x20, 0x0A, 0x20, 0x0C, 0x20, 0x0D, 0x20,
            0x09, 0x20, 0x41
        ]
    );

    let source = r#"WSTRING#"$$ $" $L $N $P $R $T $0041 $03B1""#;
    assert_eq!(
        string_units(source),
        vec![
            0x24, 0x20, 0x22, 0x20, 0x0A, 0x20, 0x0A, 0x20, 0x0C, 0x20, 0x0D, 0x20, 0x09, 0x20,
            0x41, 0x20, 0x03B1
        ]
    );

    // Either quote escape is accepted in either delimiter.
    assert_eq!(string_units(r#"WSTRING#"$'""#), vec![0x27]);
}

#[test]
fn stc_lex_028_wrong_hex_escape_width() {
    assert_code("'$0041'", stc_lexer::codes::INVALID_ESCAPE);
    assert_code("WSTRING#\"$41\"", stc_lexer::codes::INVALID_ESCAPE);
    assert_code("'$4'", stc_lexer::codes::INVALID_ESCAPE);
    assert_code("'abc$", stc_lexer::codes::INVALID_ESCAPE);
    assert_code("'abc$", stc_lexer::codes::UNTERMINATED_STRING);
    assert_eq!(string_units("'abc$$'"), vec![0x61, 0x62, 0x63, 0x24]);
    assert_eq!(string_units("'$41$42'"), vec![0x41, 0x42]);
}

#[test]
fn stc_lex_029_raw_newlines_and_unterminated_strings() {
    let source = "'a\nb'";
    let (stream, sink) = lex(source);
    assert!(has_code(&sink, stc_lexer::codes::INVALID_CHARACTER));
    let token = &tokens_without_eof(&stream)[0];
    assert_eq!(
        token.kind,
        TokenKind::StringLiteral {
            encoding: StringEncoding::Sbcs,
            terminated: true,
        }
    );
    assert_eq!(string_units(source), vec![0x61, 0x62]);

    let source = "'abc";
    let (stream, sink) = lex(source);
    assert!(has_code(&sink, stc_lexer::codes::UNTERMINATED_STRING));
    assert_eq!(
        tokens_without_eof(&stream)[0].kind,
        TokenKind::StringLiteral {
            encoding: StringEncoding::Sbcs,
            terminated: false,
        }
    );
}

#[test]
fn stc_lex_030_typed_string_and_character_literals() {
    assert_eq!(
        literal("STRING#'a'"),
        Literal::String(stc_lexer::StringLiteral {
            encoding: StringEncoding::Sbcs,
            delimiter: '\'',
            units: vec![0x61],
        })
    );
    assert_eq!(
        literal("WSTRING#\"A\""),
        Literal::String(stc_lexer::StringLiteral {
            encoding: StringEncoding::Dbcs,
            delimiter: '"',
            units: vec![0x41],
        })
    );
    assert_eq!(
        literal("CHAR#'A'"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::Char,
            value: Some(0x41),
        })
    );
    assert_eq!(
        literal("CHAR#\"A\""),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::Char,
            value: Some(0x41),
        })
    );
    assert_eq!(
        literal("WCHAR#\"A\""),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::WChar,
            value: Some(0x41),
        })
    );
    assert_eq!(
        literal("WCHAR#'A'"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::WChar,
            value: Some(0x41),
        })
    );
    assert_eq!(
        literal("UCHAR#'A'"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::UChar,
            value: Some(0x41),
        })
    );

    assert_code("STRING#123", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("CHAR#'ab'", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("WCHAR#\"😀\"", stc_lexer::codes::MALFORMED_LITERAL);
}

#[test]
fn stc_lex_031_empty_strings_and_characters() {
    assert_eq!(string_units("''"), Vec::<u32>::new());
    assert_eq!(string_units("\"\""), Vec::<u32>::new());
    assert_code("CHAR#''", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("UCHAR#\"\"", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("WCHAR#''", stc_lexer::codes::MALFORMED_LITERAL);
}

#[test]
fn stc_lex_032_u_string_forms() {
    for source in ["U#'abc'", "U#\"abc\"", "USTRING#'abc'", "USTRING#\"abc\""] {
        let token = first_token(source);
        assert_eq!(
            token.kind,
            TokenKind::UStringLiteral { terminated: true },
            "{source}"
        );
        assert_eq!(string_units(source), vec![0x61, 0x62, 0x63]);
    }
    assert_eq!(string_units("U#'é😀'"), vec![0xE9, 0x1F600]);
}

#[test]
fn stc_lex_033_raw_unicode_in_u_strings() {
    assert_eq!(string_units("U#'é😀'"), vec![0xE9, 0x1F600]);
    assert_eq!(string_units("U#'\u{feff}'"), vec![0xFEFF]);
    assert_code("U#'a\0b'", stc_lexer::codes::INVALID_CHARACTER);
}

#[test]
fn stc_lex_034_u_string_escapes() {
    let source = r#"U#"$$ $" $L $N $P $R $T $41 ${1F600} $0041""#;
    assert_eq!(
        string_units(source),
        vec![
            0x24, 0x20, 0x22, 0x20, 0x0A, 0x20, 0x0A, 0x20, 0x0C, 0x20, 0x0D, 0x20, 0x09, 0x20,
            0x41, 0x20, 0x1F600, 0x20, 0x41
        ]
    );
    assert_eq!(string_units("U#'${41}'"), vec![0x41]);
    assert_eq!(string_units("U#'$41'"), vec![0x41]);
    assert_code("'${41}'", stc_lexer::codes::INVALID_ESCAPE);
}

#[test]
fn stc_lex_035_uchar_literals() {
    assert_eq!(
        literal("UCHAR#${1F600}"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::UChar,
            value: Some(0x1F600),
        })
    );
    assert_eq!(
        literal("UCHAR#$41"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::UChar,
            value: Some(0x41),
        })
    );
    assert_eq!(
        literal("UCHAR#$0041"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::UChar,
            value: Some(0x41),
        })
    );
    assert_eq!(
        literal("UCHAR#$$"),
        Literal::Char(stc_lexer::CharLiteral {
            kind: CharKind::UChar,
            value: Some(0x24),
        })
    );
    assert_code("UCHAR#", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("UCHAR#''", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("UCHAR#'ab'", stc_lexer::codes::MALFORMED_LITERAL);
    assert_code("UCHAR#${D800}", stc_lexer::codes::INVALID_SCALAR_ESCAPE);
}

#[test]
fn stc_lex_036_invalid_scalar_escapes() {
    for source in [
        r"U#'${}'",
        r"U#'${1234567}'",
        r"U#'${110000}'",
        r"U#'${D800}'",
        r"U#'$D800'",
        r"U#'${zz}'",
        r"U#'${1234'",
    ] {
        assert_code(source, stc_lexer::codes::INVALID_SCALAR_ESCAPE);
    }
}

#[test]
fn stc_lex_037_no_unicode_normalization() {
    let composed = string_units("U#'é'");
    let decomposed = string_units("U#'e\u{301}'");
    assert_eq!(composed, vec![0xE9]);
    assert_eq!(decomposed, vec![0x65, 0x301]);
    assert_ne!(composed, decomposed);
}

#[test]
fn stc_lex_038_duration_literals() {
    let duration = |source: &str| match literal(source) {
        Literal::Duration(duration) => duration,
        other => panic!("expected duration for {source}, got {other:?}"),
    };

    let short = duration("T#14ms");
    assert!(!short.long);
    assert_eq!(short.sign, None);
    assert_eq!(
        short.components,
        vec![DurationComponent {
            value: "14".into(),
            fraction: None,
            unit: TimeUnit::Millisecond,
        }]
    );

    let long = duration("TIME#25h_15m");
    assert!(!long.long);
    assert_eq!(long.components.len(), 2);
    assert_eq!(long.components[0].unit, TimeUnit::Hour);
    assert_eq!(long.components[1].unit, TimeUnit::Minute);

    let long = duration("LTIME#5d_14h_12m_18s_3.5ms");
    assert!(long.long);
    assert_eq!(long.components.len(), 5);
    assert_eq!(long.components[4].unit, TimeUnit::Millisecond);
    assert_eq!(long.components[4].fraction.as_deref(), Some("5"));

    assert_eq!(duration("T#-5s").sign, Some(Sign::Minus));
    assert_eq!(duration("T#+1d").sign, Some(Sign::Plus));
    assert_eq!(duration("T#5m30s").components.len(), 2);
    assert_eq!(duration("T#1H_2MS").components[0].unit, TimeUnit::Hour);
    assert_eq!(
        duration("T#1H_2MS").components[1].unit,
        TimeUnit::Millisecond
    );

    for invalid in [
        "T#",
        "T#1x",
        "T#1_000ms",
        "T#1.5s_2ms",
        "T#1.5_0s",
        "T#1s_",
        "LTIME#",
    ] {
        assert_code(invalid, stc_lexer::codes::MALFORMED_LITERAL);
    }
}

#[test]
fn stc_lex_040_date_and_time_prefixes() {
    let date = |source: &str| match literal(source) {
        Literal::Date(date) => date,
        other => panic!("expected date for {source}, got {other:?}"),
    };
    let tod = |source: &str| match literal(source) {
        Literal::Tod(tod) => tod,
        other => panic!("expected time-of-day for {source}, got {other:?}"),
    };
    let datetime = |source: &str| match literal(source) {
        Literal::DateTime(datetime) => datetime,
        other => panic!("expected date-time for {source}, got {other:?}"),
    };

    assert!(!date("DATE#1984-06-25").long);
    assert!(!date("D#1984-06-25").long);
    assert!(date("LDATE#1984-06-25").long);
    assert!(date("LD#1984-06-25").long);
    assert_eq!(date("D#1984-06-25").year, 1984);
    assert_eq!(date("D#1984-06-25").month, 6);
    assert_eq!(date("D#1984-06-25").day, 25);

    assert!(!tod("TIME_OF_DAY#15:36:55").long);
    assert!(!tod("TOD#15:36:55").long);
    assert!(tod("LTOD#15:36:55").long);
    assert!(tod("LTIME_OF_DAY#15:36:55").long);
    assert_eq!(tod("TOD#15:36:55").hour, 15);
    assert_eq!(tod("TOD#15:36:55").minute, 36);
    assert_eq!(tod("TOD#15:36:55").second, 55);

    assert!(!datetime("DATE_AND_TIME#1984-06-25-15:36:55").long);
    assert!(!datetime("DT#1984-06-25-15:36:55").long);
    assert!(datetime("LDT#1984-06-25-15:36:55").long);
    assert!(datetime("LDATE_AND_TIME#1984-06-25-15:36:55").long);
}

#[test]
fn stc_lex_041_date_and_time_shapes() {
    let tod = |source: &str| match literal(source) {
        Literal::Tod(tod) => tod,
        other => panic!("expected time-of-day for {source}, got {other:?}"),
    };
    assert_eq!(tod("TOD#15:36:55.36").fraction.as_deref(), Some("36"));
    assert_eq!(
        tod("LTOD#15:36:55.360227400").fraction.as_deref(),
        Some("360227400")
    );
    assert_eq!(tod("TOD#15:36:55.3_6").fraction.as_deref(), Some("36"));

    for invalid in [
        "D#1984-6-25",
        "D#1984-06-2",
        "TOD#1:2:3",
        "TOD#15:36",
        "DT#1984-06-25",
        "TOD#15:36:55.1234567890",
    ] {
        assert_code(invalid, stc_lexer::codes::MALFORMED_LITERAL);
    }
}

#[test]
fn stc_lex_042_range_validation_is_deferred() {
    let date = match literal("D#2024-99-99") {
        Literal::Date(date) => date,
        other => panic!("expected date, got {other:?}"),
    };
    assert_eq!((date.year, date.month, date.day), (2024, 99, 99));
    assert!(lex("D#2024-99-99").1.is_empty());

    let tod = match literal("TOD#99:99:99") {
        Literal::Tod(tod) => tod,
        other => panic!("expected time-of-day, got {other:?}"),
    };
    assert_eq!((tod.hour, tod.minute, tod.second), (99, 99, 99));
    assert!(lex("TOD#99:99:99").1.is_empty());
}
