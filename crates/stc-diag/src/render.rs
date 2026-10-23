use std::fmt::Write as _;

use stc_span::{SourceMap, Span};

use crate::Diagnostic;

/// Renders one diagnostic in the human-readable format used by the CLI:
///
/// ```text
/// src/main.st:1:9: error[STC-E-LEX-002]: invalid character '@'
///   |
/// 1 | var @ x;
///   |     ^
///   |
///   = note: ...
/// ```
///
/// The returned string ends with a newline. Colors are not applied here; the
/// driver adds them when the terminal supports it.
#[must_use]
pub fn render_human(diagnostic: &Diagnostic, sources: &SourceMap) -> String {
    let location = sources.location(diagnostic.primary);
    let path = sources.file_path(location.file).unwrap_or("<unknown>");
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{path}:{}:{}: {}[{}]: {}",
        location.line, location.column, diagnostic.severity, diagnostic.code, diagnostic.message
    );

    let primary_label = diagnostic
        .labels
        .iter()
        .find(|label| label.span == diagnostic.primary)
        .map(|label| label.message.as_str());
    render_snippet(&mut out, sources, diagnostic.primary, primary_label);

    for label in &diagnostic.labels {
        if label.span != diagnostic.primary {
            render_snippet(&mut out, sources, label.span, Some(&label.message));
        }
    }

    let width = location.line.to_string().len();
    if !diagnostic.notes.is_empty() || !diagnostic.suggestions.is_empty() {
        let _ = writeln!(out, "{:width$} |", "");
        for note in &diagnostic.notes {
            let _ = writeln!(out, "{:width$} = note: {note}", "");
        }
        for suggestion in &diagnostic.suggestions {
            let _ = write!(out, "{:width$} = help: {}", "", suggestion.message);
            if !suggestion.replacement.is_empty() {
                let _ = write!(out, ": `{}`", suggestion.replacement);
            }
            out.push('\n');
        }
    }
    out
}

fn render_snippet(out: &mut String, sources: &SourceMap, span: Span, label: Option<&str>) {
    let location = sources.location(span);
    let line_text = sources.line_snippet(span);
    let width = location.line.to_string().len();
    let _ = writeln!(out, "{:width$} |", "");
    let _ = writeln!(out, "{:>width$} | {line_text}", location.line);

    let line_start = sources
        .file(location.file)
        .and_then(|file| file.line_start(location.line))
        .unwrap_or(span.start);
    let line_end = line_start.saturating_add(line_text.len() as u32);
    let start = span.start.clamp(line_start, line_end);
    let end = span.end.clamp(start, line_end);
    let pad = sources
        .text(Span::new(span.file, line_start, start))
        .chars()
        .count();
    let carets = sources
        .text(Span::new(span.file, start, end))
        .chars()
        .count()
        .max(1);

    let _ = write!(out, "{:width$} | ", "");
    for _ in 0..pad {
        out.push(' ');
    }
    for _ in 0..carets {
        out.push('^');
    }
    if let Some(label) = label {
        out.push(' ');
        out.push_str(label);
    }
    out.push('\n');
}

/// Renders one diagnostic as a single-line JSON object, as required by
/// `--message-format=json`.
///
/// The returned string does not include a trailing newline.
#[must_use]
pub fn render_json(diagnostic: &Diagnostic, sources: &SourceMap) -> String {
    let location = sources.location(diagnostic.primary);
    let path = sources.file_path(location.file).unwrap_or("<unknown>");
    let mut out = String::new();
    out.push('{');
    let _ = write!(
        out,
        "\"code\":{},\"severity\":{},\"message\":{},\"file\":{}",
        json_string(diagnostic.code),
        json_string(diagnostic.severity.as_str()),
        json_string(&diagnostic.message),
        json_string(path)
    );
    let _ = write!(
        out,
        ",\"span\":{{\"start\":{},\"end\":{},\"line\":{},\"column\":{}}}",
        diagnostic.primary.start, diagnostic.primary.end, location.line, location.column
    );
    out.push_str(",\"labels\":[");
    for (index, label) in diagnostic.labels.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"span\":{{\"start\":{},\"end\":{}}},\"message\":{}}}",
            label.span.start,
            label.span.end,
            json_string(&label.message)
        );
    }
    out.push_str("],\"notes\":[");
    for (index, note) in diagnostic.notes.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&json_string(note));
    }
    out.push_str("],\"suggestions\":[");
    for (index, suggestion) in diagnostic.suggestions.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _ = write!(
            out,
            "{{\"message\":{},\"span\":{{\"start\":{},\"end\":{}}},\"replacement\":{}}}",
            json_string(&suggestion.message),
            suggestion.span.start,
            suggestion.span.end,
            json_string(&suggestion.replacement)
        );
    }
    out.push_str("]}");
    out
}

fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            character if (character as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", character as u32);
            }
            character => out.push(character),
        }
    }
    out.push('"');
    out
}
