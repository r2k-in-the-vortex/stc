mod common;

use common::*;
use stc_diag::Severity;
use stc_lexer::{Keyword, Literal, Punct, StringEncoding, TokenKind, TriviaKind};
use stc_span::SourceMap;

#[test]
fn stc_lex_001_initial_bom_is_trivia() {
    let (stream, sink) = lex("\u{feff}PROGRAM");
    assert!(sink.is_empty(), "{:?}", sink.diagnostics());
    assert_eq!(stream.trivia[0].kind, TriviaKind::Bom);
    assert_eq!(stream.trivia[0].span.start, 0);
    assert_eq!(stream.trivia[0].span.end, 3);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens[0].kind, TokenKind::Keyword(Keyword::Program));
    assert_eq!(tokens[0].span.start, 3);
}

#[test]
fn stc_lex_002_invalid_utf8_is_reported_and_recovered() {
    let (lexed, sink) = lex_bytes(b"F\xffX");
    assert!(has_code(&sink, stc_lexer::codes::INVALID_UTF8));
    assert_eq!(lexed.text, "F\u{fffd}X");
    assert_eq!(lexed.stream.reconstruct(&lexed.text), lexed.text);
}

#[test]
fn stc_lex_003_bom_elsewhere_nul_and_surrogates() {
    assert_code("A\u{feff}", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("\0", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("'a\0b'", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("// a\0b", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("(* a\0b *)", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("{stc:x \0}", stc_lexer::codes::INVALID_CHARACTER);

    let (_, sink) = lex_bytes(&[0xed, 0xa0, 0x80]);
    assert!(has_code(&sink, stc_lexer::codes::INVALID_UTF8));
}

#[test]
fn stc_lex_004_every_byte_is_preserved() {
    let source = "\u{feff}// lead\nVAR {stc:test} x : INT := 16#1_F;\r\n\
                  (* nested (* comment *) *)\n\
                  U#'héllo ${1F600}' // tail\rEND_VAR";
    let (stream, sink) = lex(source);
    assert!(sink.is_empty(), "{:?}", sink.diagnostics());
    assert_eq!(stream.reconstruct(source), source);
    let covered: u32 = stream
        .tokens
        .iter()
        .map(|token| token.span.len())
        .sum::<u32>()
        + stream
            .trivia
            .iter()
            .map(|trivia| trivia.span.len())
            .sum::<u32>();
    assert_eq!(covered, source.len() as u32);
}

#[test]
fn stc_lex_005_line_terminators_and_columns() {
    let source = "a\nb\r\nc\rd";
    let (stream, _) = lex(source);
    let mut map = SourceMap::new();
    let file = map.add_file("terminators.st", source);
    for (token, line) in tokens_without_eof(&stream).iter().zip(1..=4) {
        let location = map.location(token.span);
        assert_eq!(location.file, file);
        assert_eq!((location.line, location.column), (line, 1), "{source:?}");
    }

    // Form feed terminates a line comment but is horizontal whitespace.
    let source = "//x\u{0c}y";
    let (stream, sink) = lex(source);
    assert!(sink.is_empty());
    assert_eq!(stream.trivia[0].kind, TriviaKind::LineComment);
    assert_eq!(stream.trivia[0].text(source), "//x");
    assert_eq!(stream.trivia[1].kind, TriviaKind::Whitespace);
    assert_eq!(texts(source), vec!["y"]);
}

#[test]
fn stc_lex_006_byte_offsets_follow_utf8() {
    let source = "// é\nx";
    let (stream, _) = lex(source);
    let x = &tokens_without_eof(&stream)[0];
    assert_eq!(x.span.start, 6);
    assert_eq!(x.span.end, 7);

    let mut map = SourceMap::new();
    let file = map.add_file("utf8.st", source);
    let location = map.location(x.span);
    assert_eq!(
        (location.file, location.line, location.column),
        (file, 2, 1)
    );
}

#[test]
fn stc_lex_007_source_map_owns_text_and_locations() {
    let source = "PROGRAM Main\nEND_PROGRAM\n";
    let (stream, _) = lex(source);
    let mut map = SourceMap::new();
    let file = map.add_file("main.st", source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(map.text(tokens[0].span), "PROGRAM");
    assert_eq!(map.line_snippet(tokens[2].span), "END_PROGRAM");
    assert_eq!(map.file_path(file), Some("main.st"));
    assert_eq!(map.file(file).unwrap().line_count(), 3);
}

#[test]
fn stc_lex_008_character_classes() {
    let stream = lex_clean("a1_Z\t0 _x");
    let kinds = tokens_without_eof(&stream)
        .iter()
        .map(|token| token.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident("A1_Z".into()),
            TokenKind::IntLiteral,
            TokenKind::Ident("_X".into()),
        ]
    );
}

#[test]
fn stc_lex_009_keywords_and_identifiers_are_case_insensitive() {
    let stream = lex_clean("if IF If motor_1 MOTOR_1 Motor_1");
    let tokens = tokens_without_eof(&stream);
    for token in &tokens[..3] {
        assert_eq!(token.kind, TokenKind::Keyword(Keyword::If));
        assert_eq!(token.normalized(), Some("IF"));
    }
    for token in &tokens[3..] {
        assert_eq!(token.normalized(), Some("MOTOR_1"));
    }
}

#[test]
fn stc_lex_010_whitespace_is_trivia() {
    let source = " \t\u{0b}\u{0c}\n\r\n\r";
    let stream = lex_clean(source);
    assert!(tokens_without_eof(&stream).is_empty());
    assert_eq!(stream.trivia.len(), 4);
    assert!(
        stream
            .trivia
            .iter()
            .all(|trivia| trivia.kind == TriviaKind::Whitespace)
    );
}

#[test]
fn stc_lex_011_non_ascii_is_allowed_only_in_comments_and_strings() {
    let source = "// héllo\n(* 日本語 *)\nWSTRING#\"日本\"\nU#'😀'";
    let stream = lex_clean(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(
        tokens[0].kind,
        TokenKind::StringLiteral {
            encoding: StringEncoding::Dbcs,
            terminated: true,
        }
    );
    assert_eq!(
        tokens[1].kind,
        TokenKind::UStringLiteral { terminated: true }
    );

    assert_code("é", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("aé", stc_lexer::codes::INVALID_CHARACTER);
}

#[test]
fn stc_lex_012_identifier_shape() {
    let stream = lex_clean("_a1 a_1");
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens[0].kind, TokenKind::Ident("_A1".into()));
    assert_eq!(tokens[1].kind, TokenKind::Ident("A_1".into()));

    let stream = lex_clean("1abc");
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens[0].kind, TokenKind::IntLiteral);
    assert_eq!(tokens[1].kind, TokenKind::Ident("ABC".into()));
}

#[test]
fn stc_lex_013_identifier_underscore_rules() {
    let stream = lex_clean("_MAIN LIM_SW_5");
    assert_eq!(tokens_without_eof(&stream).len(), 2);
    for invalid in ["__LIM", "LIM__SW5", "LIM_SW5_", "_"] {
        assert_code(invalid, stc_lexer::codes::INVALID_IDENTIFIER);
    }
}

#[test]
fn stc_lex_014_original_spelling_is_preserved() {
    let source = "abc ABC aBc";
    let stream = lex_clean(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens[0].text(source), "abc");
    assert_eq!(tokens[1].text(source), "ABC");
    assert_eq!(tokens[2].text(source), "aBc");
    assert_eq!(tokens[0].normalized(), Some("ABC"));
    assert_eq!(tokens[1].normalized(), Some("ABC"));
    assert_eq!(tokens[2].normalized(), Some("ABC"));
}

#[test]
fn stc_lex_015_unicode_letters_are_invalid() {
    assert_code("π", stc_lexer::codes::INVALID_CHARACTER);
    assert_code("aπ", stc_lexer::codes::INVALID_CHARACTER);
}

#[test]
fn stc_lex_016_identifier_length_limit() {
    let longest = "a".repeat(1024);
    let stream = lex_clean(&longest);
    assert_eq!(
        tokens_without_eof(&stream)[0].kind,
        TokenKind::Ident(longest.to_ascii_uppercase().into())
    );
    assert_code(&"a".repeat(1025), stc_lexer::codes::INVALID_IDENTIFIER);
    let stream = lex_clean(&"a".repeat(255));
    assert_eq!(tokens_without_eof(&stream).len(), 1);
}

// STC-LEX-017, STC-LEX-018, STC-LEX-020–STC-LEX-023 are covered in literals.rs.

#[test]
fn stc_lex_019_signs_are_unary_outside_typed_literals() {
    assert_eq!(
        clean_kinds("-1 +1"),
        vec![
            TokenKind::Punct(Punct::Minus),
            TokenKind::IntLiteral,
            TokenKind::Punct(Punct::Plus),
            TokenKind::IntLiteral,
        ]
    );

    let token = first_token("INT#-123");
    let Literal::Typed(typed) = token.literal.as_deref().unwrap() else {
        panic!("expected a typed literal");
    };
    assert_eq!(typed.sign, Some(stc_lexer::Sign::Minus));
    assert_eq!(typed.ty, stc_lexer::ElementaryType::Int);
}

// STC-LEX-024 is covered in literals.rs.

#[test]
fn stc_lex_025_string_delimiters() {
    let cases = [
        ("'abc'", StringEncoding::Sbcs),
        ("\"abc\"", StringEncoding::Dbcs),
        ("STRING#'abc'", StringEncoding::Sbcs),
        ("STRING#\"abc\"", StringEncoding::Sbcs),
        ("WSTRING#\"abc\"", StringEncoding::Dbcs),
        ("WSTRING#'abc'", StringEncoding::Dbcs),
    ];
    for (source, encoding) in cases {
        let token = first_token(source);
        assert_eq!(
            token.kind,
            TokenKind::StringLiteral {
                encoding,
                terminated: true,
            },
            "{source}"
        );
    }
}

// STC-LEX-026–STC-LEX-037 are covered in literals.rs.

#[test]
fn stc_lex_039_prefix_and_hash_must_be_adjacent() {
    let source = "T #1s TIME #1s";
    let kinds = clean_kinds(source);
    assert_eq!(
        kinds,
        vec![
            TokenKind::Ident("T".into()),
            TokenKind::Punct(Punct::Hash),
            TokenKind::IntLiteral,
            TokenKind::Ident("S".into()),
            TokenKind::Keyword(Keyword::Time),
            TokenKind::Punct(Punct::Hash),
            TokenKind::IntLiteral,
            TokenKind::Ident("S".into()),
        ]
    );
}

// STC-LEX-038, STC-LEX-040–STC-LEX-042 are covered in literals.rs.

#[test]
fn stc_lex_043_direct_variable_shape() {
    let cases = [
        ("%IX0.0", Some("I"), Some("X"), vec![0, 0], false),
        ("%QW28", Some("Q"), Some("W"), vec![28], false),
        ("%MD100", Some("M"), Some("D"), vec![100], false),
        ("%0", None, None, vec![0], false),
        ("%B0", None, Some("B"), vec![0], false),
        ("%ix0.0", Some("I"), Some("X"), vec![0, 0], false),
    ];
    for (source, area, size, indices, wildcard) in cases {
        let token = first_token(source);
        assert_eq!(token.kind, TokenKind::DirectVariable, "{source}");
        let Literal::Direct(direct) = token.literal.as_deref().unwrap() else {
            panic!("expected direct variable for {source}");
        };
        assert_eq!(
            direct.area.map(|area| area.as_char().to_string()),
            area.map(str::to_string)
        );
        assert_eq!(
            direct.size.map(|size| size.as_char().to_string()),
            size.map(str::to_string)
        );
        assert_eq!(direct.indices, indices);
        assert_eq!(direct.wildcard, wildcard);
    }

    let token = first_token("%IX4294967295");
    let Literal::Direct(direct) = token.literal.as_deref().unwrap() else {
        panic!("expected direct variable");
    };
    assert_eq!(direct.indices, vec![u32::MAX]);
    assert_code("%IX4294967296", stc_lexer::codes::MALFORMED_LITERAL);
}

#[test]
fn stc_lex_044_wildcard_direct_variables() {
    for (source, area) in [("%I*", "I"), ("%Q*", "Q"), ("%M*", "M")] {
        let token = first_token(source);
        let Literal::Direct(direct) = token.literal.as_deref().unwrap() else {
            panic!("expected direct variable for {source}");
        };
        assert!(direct.wildcard);
        assert_eq!(direct.area.unwrap().as_char().to_string(), area);
        assert!(direct.indices.is_empty());
    }

    // A size letter is not allowed in a wildcard form.
    assert_eq!(
        clean_kinds("%IX*"),
        vec![
            TokenKind::Punct(Punct::Percent),
            TokenKind::Ident("IX".into()),
            TokenKind::Punct(Punct::Star),
        ]
    );
}

#[test]
fn stc_lex_045_struct_relative_and_multi_index_addresses() {
    let token = first_token("%IX0.1.2");
    let Literal::Direct(direct) = token.literal.as_deref().unwrap() else {
        panic!("expected direct variable");
    };
    assert_eq!(direct.indices, vec![0, 1, 2]);
}

#[test]
fn stc_lex_046_percent_falls_back_to_punctuation() {
    assert_eq!(clean_kinds("%"), vec![TokenKind::Punct(Punct::Percent)]);
    assert_eq!(
        clean_kinds("%Z"),
        vec![
            TokenKind::Punct(Punct::Percent),
            TokenKind::Ident("Z".into()),
        ]
    );
    assert_eq!(
        clean_kinds("a.%X0"),
        vec![
            TokenKind::Ident("A".into()),
            TokenKind::Punct(Punct::Dot),
            TokenKind::DirectVariable,
        ]
    );
}

#[test]
fn stc_lex_047_line_comments() {
    let source = "//x (* /*\ny";
    let (stream, sink) = lex(source);
    assert!(sink.is_empty());
    assert_eq!(stream.trivia[0].kind, TriviaKind::LineComment);
    assert_eq!(stream.trivia[0].text(source), "//x (* /*");
    assert_eq!(texts(source), vec!["y"]);
}

#[test]
fn stc_lex_048_block_comments_nest_within_one_style() {
    let source = "(* a (* b *) c *) /* a /* b */ c */";
    let (stream, sink) = lex(source);
    assert!(sink.is_empty());
    let block_comments = stream
        .trivia
        .iter()
        .filter(|trivia| trivia.kind == TriviaKind::BlockComment)
        .count();
    assert_eq!(block_comments, 2);
    assert!(tokens_without_eof(&stream).is_empty());

    let cross = "(* /* *)\n/* (* */";
    let (stream, sink) = lex(cross);
    assert!(sink.is_empty());
    assert_eq!(stream.trivia[0].text(cross), "(* /* *)");
    assert_eq!(stream.trivia[2].text(cross), "/* (* */");
}

#[test]
fn stc_lex_049_unterminated_block_comment() {
    let source = "(* abc";
    let (stream, sink) = lex(source);
    assert!(has_code(
        &sink,
        stc_lexer::codes::UNTERMINATED_BLOCK_COMMENT
    ));
    assert_eq!(stream.trivia[0].kind, TriviaKind::BlockComment);
    assert_eq!(stream.trivia[0].span.end, 6);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens.len(), 1);
    assert!(tokens[0].kind.is_error());
    assert_eq!(tokens[0].span.start, 6);

    let source = "x (* abc";
    let (stream, sink) = lex(source);
    assert!(has_code(
        &sink,
        stc_lexer::codes::UNTERMINATED_BLOCK_COMMENT
    ));
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens[0].kind, TokenKind::Ident("X".into()));
    assert_eq!(stream.trailing_trivia(&tokens[0]).len(), 2);
    assert!(
        stream
            .trailing_trivia(&tokens[0])
            .iter()
            .any(|trivia| trivia.kind == TriviaKind::BlockComment)
    );
    assert!(tokens[1].kind.is_error());
}

#[test]
fn stc_lex_050_comments_are_trivia() {
    let source = "a//b\nc";
    let (stream, _) = lex(source);
    assert_eq!(
        tokens_without_eof(&stream)
            .iter()
            .map(|token| token.kind.clone())
            .collect::<Vec<_>>(),
        vec![TokenKind::Ident("A".into()), TokenKind::Ident("C".into())]
    );
    assert_eq!(stream.trivia[0].kind, TriviaKind::LineComment);
    assert_eq!(stream.trivia[1].kind, TriviaKind::Whitespace);
}

#[test]
fn stc_lex_051_documentation_line_comments() {
    let source = "/// doc\n//// more\n// plain";
    let (stream, _) = lex(source);
    let kinds = stream
        .trivia
        .iter()
        .map(|trivia| trivia.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            TriviaKind::DocComment,
            TriviaKind::Whitespace,
            TriviaKind::DocComment,
            TriviaKind::Whitespace,
            TriviaKind::LineComment,
        ]
    );
}

#[test]
fn stc_lex_052_pragmas_track_balanced_braces_and_strings() {
    let source = r#"{stc:io {"device":"d1","nested":{"a":1}}}"#;
    let stream = lex_clean(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Pragma { terminated: true });
    assert_eq!(tokens[0].text(source), source);

    let single = "{x '}'}";
    let stream = lex_clean(single);
    assert_eq!(tokens_without_eof(&stream).len(), 1);

    let escaped = r#"{x "a\"}b"}"#;
    let stream = lex_clean(escaped);
    assert_eq!(tokens_without_eof(&stream).len(), 1);

    let multiline = "{\nstc:test\n}";
    let stream = lex_clean(multiline);
    assert_eq!(tokens_without_eof(&stream).len(), 1);
}

#[test]
fn stc_lex_053_pragmas_are_tokens_between_other_tokens() {
    let source = "VAR {stc:test} x : INT;";
    assert_eq!(
        clean_kinds(source),
        vec![
            TokenKind::Keyword(Keyword::Var),
            TokenKind::Pragma { terminated: true },
            TokenKind::Ident("X".into()),
            TokenKind::Punct(Punct::Colon),
            TokenKind::Keyword(Keyword::Int),
            TokenKind::Punct(Punct::Semicolon),
        ]
    );
}

#[test]
fn stc_lex_054_pragma_contents_are_opaque_to_the_lexer() {
    let cases = [
        r#"{stc:unknown {"x":1}}"#,
        "{attribute 'something'}",
        "{not-stc at all}",
        "{stc:x \"\u{feff}\"}",
    ];
    for source in cases {
        let stream = lex_clean(source);
        let tokens = tokens_without_eof(&stream);
        assert_eq!(tokens.len(), 1, "{source}");
        assert_eq!(tokens[0].text(source), source);
    }
}

#[test]
fn stc_lex_055_unterminated_pragma() {
    let source = "{stc:test";
    let (stream, sink) = lex(source);
    assert!(has_code(&sink, stc_lexer::codes::UNTERMINATED_PRAGMA));
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens[0].kind, TokenKind::Pragma { terminated: false });
    assert_eq!(tokens[0].span.end as usize, source.len());
}

#[test]
fn stc_lex_056_json_payload_is_not_lexed_as_st() {
    let source = "{stc:test {not json}}";
    let stream = lex_clean(source);
    assert_eq!(tokens_without_eof(&stream).len(), 1);
    assert_eq!(tokens_without_eof(&stream)[0].text(source), source);
}

#[test]
fn stc_lex_057_pragma_payload_presence_is_not_lexed() {
    let stream = lex_clean("{stc:export}");
    assert_eq!(
        tokens_without_eof(&stream)[0].kind,
        TokenKind::Pragma { terminated: true }
    );
}

#[test]
fn stc_lex_058_legacy_parenthesized_pragma_is_one_token() {
    let source = "{stc:test(cycles := 25)}";
    let stream = lex_clean(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].text(source), source);
}

#[test]
fn punctuation_uses_maximal_munch() {
    let source = ":= => ?= ** .. <> <= >= : . < > = + - * / ( ) [ ] ; , # @ ^ & %";
    assert_eq!(
        clean_kinds(source),
        vec![
            TokenKind::Punct(Punct::Assign),
            TokenKind::Punct(Punct::OutputAssign),
            TokenKind::Punct(Punct::AttemptAssign),
            TokenKind::Punct(Punct::Power),
            TokenKind::Punct(Punct::Range),
            TokenKind::Punct(Punct::NotEqual),
            TokenKind::Punct(Punct::LessEqual),
            TokenKind::Punct(Punct::GreaterEqual),
            TokenKind::Punct(Punct::Colon),
            TokenKind::Punct(Punct::Dot),
            TokenKind::Punct(Punct::Less),
            TokenKind::Punct(Punct::Greater),
            TokenKind::Punct(Punct::Equal),
            TokenKind::Punct(Punct::Plus),
            TokenKind::Punct(Punct::Minus),
            TokenKind::Punct(Punct::Star),
            TokenKind::Punct(Punct::Slash),
            TokenKind::Punct(Punct::LParen),
            TokenKind::Punct(Punct::RParen),
            TokenKind::Punct(Punct::LBracket),
            TokenKind::Punct(Punct::RBracket),
            TokenKind::Punct(Punct::Semicolon),
            TokenKind::Punct(Punct::Comma),
            TokenKind::Punct(Punct::Hash),
            TokenKind::Punct(Punct::At),
            TokenKind::Punct(Punct::Caret),
            TokenKind::Punct(Punct::Ampersand),
            TokenKind::Punct(Punct::Percent),
        ]
    );
}

#[test]
fn keyword_table_is_complete_and_case_insensitive() {
    assert!(Keyword::ALL.len() >= 140, "{}", Keyword::ALL.len());
    for keyword in Keyword::ALL {
        let lower = keyword.as_str().to_ascii_lowercase();
        for spelling in [keyword.as_str(), lower.as_str()] {
            let stream = lex_clean(spelling);
            let tokens = tokens_without_eof(&stream);
            assert_eq!(tokens.len(), 1, "{spelling}");
            assert_eq!(tokens[0].kind, TokenKind::Keyword(*keyword), "{spelling}");
        }
    }
}

#[test]
fn keyword_boundaries_are_exact() {
    assert_eq!(
        clean_kinds("IFX ENDIF END END_IF IF"),
        vec![
            TokenKind::Ident("IFX".into()),
            TokenKind::Ident("ENDIF".into()),
            TokenKind::Ident("END".into()),
            TokenKind::Keyword(Keyword::EndIf),
            TokenKind::Keyword(Keyword::If),
        ]
    );
}

#[test]
fn trivia_attachment_is_same_line_only() {
    let source = "a // c\nb";
    let (stream, _) = lex(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(stream.trailing_trivia(&tokens[0]).len(), 2);
    assert!(
        stream
            .trailing_trivia(&tokens[0])
            .iter()
            .any(|trivia| trivia.kind == TriviaKind::LineComment)
    );
    assert_eq!(stream.leading_trivia(&tokens[1]).len(), 1);
    assert_eq!(
        stream.leading_trivia(&tokens[1])[0].kind,
        TriviaKind::Whitespace
    );

    let source = "a  b";
    let (stream, _) = lex(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(stream.trailing_trivia(&tokens[0]).len(), 1);
    assert!(stream.leading_trivia(&tokens[1]).is_empty());

    let source = "a (* c *) b";
    let (stream, _) = lex(source);
    let tokens = tokens_without_eof(&stream);
    assert!(
        stream
            .trailing_trivia(&tokens[0])
            .iter()
            .any(|trivia| trivia.kind == TriviaKind::BlockComment)
    );

    let source = "a (* c\n d *) b";
    let (stream, _) = lex(source);
    let tokens = tokens_without_eof(&stream);
    assert_eq!(stream.trailing_trivia(&tokens[0]).len(), 1);
    assert_eq!(
        stream.trailing_trivia(&tokens[0])[0].kind,
        TriviaKind::Whitespace
    );
    assert_eq!(
        stream.leading_trivia(&tokens[1])[0].kind,
        TriviaKind::BlockComment
    );
}

#[test]
fn invalid_characters_become_error_tokens() {
    for source in ["$", "}", "?", "!", "\\", "|", "~", "`"] {
        let (stream, sink) = lex(source);
        assert!(
            has_code(&sink, stc_lexer::codes::INVALID_CHARACTER),
            "{source:?}: {:?}",
            sink.diagnostics()
        );
        let tokens = tokens_without_eof(&stream);
        assert_eq!(tokens.len(), 1, "{source:?}");
        assert!(tokens[0].kind.is_error());
    }
}

#[test]
fn empty_and_whitespace_only_sources_have_only_eof() {
    let stream = lex_clean("");
    assert_eq!(stream.tokens.len(), 1);
    assert!(stream.tokens[0].kind.is_eof());
    assert_eq!(stream.eof_span.start, 0);
    assert_eq!(stream.eof_span.end, 0);

    let stream = lex_clean("   ");
    assert_eq!(stream.tokens.len(), 1);
    assert_eq!(stream.leading_trivia(&stream.tokens[0]).len(), 1);
}

#[test]
fn octal_warning_has_warning_severity() {
    let (_, sink) = lex("8#377");
    let warning = sink
        .iter()
        .find(|diagnostic| diagnostic.code == stc_lexer::codes::DEPRECATED_OCTAL)
        .expect("octal warning");
    assert_eq!(warning.severity, Severity::Warning);
}

#[test]
fn rule_ids_have_named_tests() {
    const TRACE: &[(&str, &str)] = &[
        ("STC-LEX-001", "stc_lex_001_initial_bom_is_trivia"),
        (
            "STC-LEX-002",
            "stc_lex_002_invalid_utf8_is_reported_and_recovered",
        ),
        (
            "STC-LEX-003",
            "stc_lex_003_bom_elsewhere_nul_and_surrogates",
        ),
        ("STC-LEX-004", "stc_lex_004_every_byte_is_preserved"),
        ("STC-LEX-005", "stc_lex_005_line_terminators_and_columns"),
        ("STC-LEX-006", "stc_lex_006_byte_offsets_follow_utf8"),
        (
            "STC-LEX-007",
            "stc_lex_007_source_map_owns_text_and_locations",
        ),
        ("STC-LEX-008", "stc_lex_008_character_classes"),
        (
            "STC-LEX-009",
            "stc_lex_009_keywords_and_identifiers_are_case_insensitive",
        ),
        ("STC-LEX-010", "stc_lex_010_whitespace_is_trivia"),
        (
            "STC-LEX-011",
            "stc_lex_011_non_ascii_is_allowed_only_in_comments_and_strings",
        ),
        ("STC-LEX-012", "stc_lex_012_identifier_shape"),
        ("STC-LEX-013", "stc_lex_013_identifier_underscore_rules"),
        ("STC-LEX-014", "stc_lex_014_original_spelling_is_preserved"),
        ("STC-LEX-015", "stc_lex_015_unicode_letters_are_invalid"),
        ("STC-LEX-016", "stc_lex_016_identifier_length_limit"),
        ("STC-LEX-017", "stc_lex_017_decimal_integer_literals"),
        ("STC-LEX-018", "stc_lex_018_real_literals"),
        (
            "STC-LEX-019",
            "stc_lex_019_signs_are_unary_outside_typed_literals",
        ),
        ("STC-LEX-020", "stc_lex_020_based_literals"),
        ("STC-LEX-021", "stc_lex_021_octal_literals_warn"),
        ("STC-LEX-022", "stc_lex_022_typed_numeric_literals"),
        (
            "STC-LEX-023",
            "stc_lex_023_raw_digits_survive_for_the_analyzer",
        ),
        ("STC-LEX-024", "stc_lex_024_boolean_literals"),
        ("STC-LEX-025", "stc_lex_025_string_delimiters"),
        (
            "STC-LEX-026",
            "stc_lex_026_raw_string_character_restrictions",
        ),
        ("STC-LEX-027", "stc_lex_027_escape_sequences"),
        ("STC-LEX-028", "stc_lex_028_wrong_hex_escape_width"),
        (
            "STC-LEX-029",
            "stc_lex_029_raw_newlines_and_unterminated_strings",
        ),
        (
            "STC-LEX-030",
            "stc_lex_030_typed_string_and_character_literals",
        ),
        ("STC-LEX-031", "stc_lex_031_empty_strings_and_characters"),
        ("STC-LEX-032", "stc_lex_032_u_string_forms"),
        ("STC-LEX-033", "stc_lex_033_raw_unicode_in_u_strings"),
        ("STC-LEX-034", "stc_lex_034_u_string_escapes"),
        ("STC-LEX-035", "stc_lex_035_uchar_literals"),
        ("STC-LEX-036", "stc_lex_036_invalid_scalar_escapes"),
        ("STC-LEX-037", "stc_lex_037_no_unicode_normalization"),
        ("STC-LEX-038", "stc_lex_038_duration_literals"),
        (
            "STC-LEX-039",
            "stc_lex_039_prefix_and_hash_must_be_adjacent",
        ),
        ("STC-LEX-040", "stc_lex_040_date_and_time_prefixes"),
        ("STC-LEX-041", "stc_lex_041_date_and_time_shapes"),
        ("STC-LEX-042", "stc_lex_042_range_validation_is_deferred"),
        ("STC-LEX-043", "stc_lex_043_direct_variable_shape"),
        ("STC-LEX-044", "stc_lex_044_wildcard_direct_variables"),
        (
            "STC-LEX-045",
            "stc_lex_045_struct_relative_and_multi_index_addresses",
        ),
        (
            "STC-LEX-046",
            "stc_lex_046_percent_falls_back_to_punctuation",
        ),
        ("STC-LEX-047", "stc_lex_047_line_comments"),
        (
            "STC-LEX-048",
            "stc_lex_048_block_comments_nest_within_one_style",
        ),
        ("STC-LEX-049", "stc_lex_049_unterminated_block_comment"),
        ("STC-LEX-050", "stc_lex_050_comments_are_trivia"),
        ("STC-LEX-051", "stc_lex_051_documentation_line_comments"),
        (
            "STC-LEX-052",
            "stc_lex_052_pragmas_track_balanced_braces_and_strings",
        ),
        (
            "STC-LEX-053",
            "stc_lex_053_pragmas_are_tokens_between_other_tokens",
        ),
        (
            "STC-LEX-054",
            "stc_lex_054_pragma_contents_are_opaque_to_the_lexer",
        ),
        ("STC-LEX-055", "stc_lex_055_unterminated_pragma"),
        ("STC-LEX-056", "stc_lex_056_json_payload_is_not_lexed_as_st"),
        (
            "STC-LEX-057",
            "stc_lex_057_pragma_payload_presence_is_not_lexed",
        ),
        (
            "STC-LEX-058",
            "stc_lex_058_legacy_parenthesized_pragma_is_one_token",
        ),
    ];
    assert_eq!(TRACE.len(), 58);
    for (index, (rule, test_name)) in TRACE.iter().enumerate() {
        assert_eq!(*rule, format!("STC-LEX-{:03}", index + 1));
        assert!(!test_name.is_empty());
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = String::new();
    for directory in ["src", "tests"] {
        collect_rust_sources(&root.join(directory), &mut sources);
    }
    for (rule, test_name) in TRACE {
        assert!(
            sources.contains(rule),
            "rule {rule} is not referenced from the crate"
        );
        assert!(
            sources.contains(test_name),
            "test {test_name} for {rule} is missing"
        );
    }
}

fn collect_rust_sources(directory: &std::path::Path, output: &mut String) {
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
