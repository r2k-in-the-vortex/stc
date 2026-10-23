use stc_span::{FileId, SourceMap, Span};

#[test]
fn span_basics() {
    let file = FileId::new(7);
    let span = Span::new(file, 3, 9);
    assert_eq!(span.len(), 6);
    assert!(!span.is_empty());
    assert!(span.contains(3));
    assert!(span.contains(8));
    assert!(!span.contains(9));
    assert_eq!(Span::empty(file, 4).len(), 0);

    let joined = Span::new(file, 3, 5).join(Span::new(file, 8, 9));
    assert_eq!(joined, Span::new(file, 3, 9));
    assert!(joined.contains_span(Span::new(file, 4, 5)));
}

#[test]
#[should_panic(expected = "cannot join spans from different files")]
fn joining_different_files_panics() {
    let _ = Span::new(FileId::new(1), 0, 1).join(Span::new(FileId::new(2), 0, 1));
}

#[test]
fn line_index_handles_all_terminators() {
    let mut map = SourceMap::new();
    let file = map.add_file("mixed.st", "a\nb\r\nc\rd\n");
    assert_eq!(map.file(file).unwrap().line_count(), 5);

    for (offset, line, column) in [(0, 1, 1), (2, 2, 1), (5, 3, 1), (7, 4, 1), (9, 5, 1)] {
        let location = map.location(Span::empty(file, offset));
        assert_eq!(
            (location.line, location.column),
            (line, column),
            "offset {offset}"
        );
    }
}

#[test]
fn line_text_strips_terminators() {
    let mut map = SourceMap::new();
    let file = map.add_file("lines.st", "one\r\ntwo\nthree\rfour");
    let source = map.file(file).unwrap();
    assert_eq!(source.line_text(1), Some("one"));
    assert_eq!(source.line_text(2), Some("two"));
    assert_eq!(source.line_text(3), Some("three"));
    assert_eq!(source.line_text(4), Some("four"));
    assert_eq!(source.line_text(5), None);
}

#[test]
fn columns_count_unicode_scalars() {
    let mut map = SourceMap::new();
    let file = map.add_file("utf8.st", "é😀x");
    assert_eq!(map.location(Span::empty(file, 0)).column, 1);
    assert_eq!(map.location(Span::empty(file, 2)).column, 2);
    assert_eq!(map.location(Span::empty(file, 6)).column, 3);

    // An offset inside a scalar is floored to the scalar start.
    assert_eq!(map.location(Span::empty(file, 1)).column, 1);
}

#[test]
fn bom_does_not_shift_the_first_column() {
    let mut map = SourceMap::new();
    let file = map.add_file("bom.st", "\u{feff}abc");
    let source = map.file(file).unwrap();
    assert_eq!(source.bom_len(), 3);
    assert_eq!(source.line_text(1), Some("abc"));
    assert_eq!(map.location(Span::empty(file, 0)).column, 1);
    assert_eq!(map.location(Span::empty(file, 3)).column, 1);
    assert_eq!(map.location(Span::empty(file, 4)).column, 2);
}

#[test]
fn text_and_snippets_use_spans() {
    let mut map = SourceMap::new();
    let file = map.add_file("src/main.st", "VAR\n  flag := counter;\nEND_VAR\n");
    let counter_start = "VAR\n  flag := ".len() as u32;
    let counter = Span::new(file, counter_start, counter_start + "counter".len() as u32);
    assert_eq!(map.text(counter), "counter");
    assert_eq!(map.line_snippet(counter), "  flag := counter;");
    let location = map.location(counter);
    assert_eq!((location.line, location.column), (2, 11));
    assert_eq!(map.file_path(file), Some("src/main.st"));
}

#[test]
fn empty_file_has_one_empty_line() {
    let mut map = SourceMap::new();
    let file = map.add_file("empty.st", "");
    let source = map.file(file).unwrap();
    assert_eq!(source.line_count(), 1);
    assert_eq!(source.line_text(1), Some(""));
    assert_eq!(map.location(Span::empty(file, 0)).line, 1);
}

#[test]
fn unknown_file_queries_are_total() {
    let map = SourceMap::new();
    let span = Span::new(FileId::new(42), 10, 20);
    assert_eq!(map.text(span), "");
    assert_eq!(map.line_snippet(span), "");
    assert_eq!(map.file_path(FileId::new(42)), None);
    let location = map.location(span);
    assert_eq!((location.line, location.column), (1, 1));
}
