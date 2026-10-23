use stc_span::{Utf8IssueKind, decode_utf8};

#[test]
fn valid_utf8_is_unchanged() {
    let decoded = decode_utf8("plain é 😀".as_bytes());
    assert_eq!(decoded.text, "plain é 😀");
    assert!(decoded.issues.is_empty());
}

#[test]
fn invalid_byte_is_replaced_and_reported() {
    let decoded = decode_utf8(b"a\xffb");
    assert_eq!(decoded.text, "a\u{fffd}b");
    assert_eq!(decoded.issues.len(), 1);
    let issue = &decoded.issues[0];
    assert_eq!(issue.byte_offset, 1);
    assert_eq!(issue.byte_len, 1);
    assert_eq!(issue.decoded_offset, 1);
    assert_eq!(issue.bytes, vec![0xff]);
    assert_eq!(issue.kind, Utf8IssueKind::InvalidSequence);
    assert_eq!(issue.hex_bytes(), "FF");
}

#[test]
fn truncated_sequence_is_reported() {
    let decoded = decode_utf8(b"a\xe2\x82");
    assert_eq!(decoded.text, "a\u{fffd}");
    assert_eq!(decoded.issues.len(), 1);
    assert_eq!(decoded.issues[0].kind, Utf8IssueKind::Truncated);
    assert_eq!(decoded.issues[0].byte_len, 2);
}

#[test]
fn multiple_invalid_sequences_are_reported_in_order() {
    let decoded = decode_utf8(b"\xffa\xc0\xaf");
    assert_eq!(decoded.text, "\u{fffd}a\u{fffd}");
    assert_eq!(decoded.issues.len(), 2);
    assert_eq!(decoded.issues[0].byte_offset, 0);
    assert_eq!(decoded.issues[1].byte_offset, 2);
    assert!(decoded.issues[1].byte_len >= 1);
}

#[test]
fn utf8_encoded_surrogates_are_invalid() {
    let decoded = decode_utf8(&[0xed, 0xa0, 0x80]);
    assert_eq!(decoded.text, "\u{fffd}");
    assert_eq!(decoded.issues.len(), 1);
}

#[test]
fn issue_display_includes_offset_and_bytes() {
    let decoded = decode_utf8(b"\xf0\x28");
    let text = decoded.issues[0].to_string();
    assert!(text.contains("byte 0"), "{text}");
    assert!(text.contains("F0"), "{text}");
}
