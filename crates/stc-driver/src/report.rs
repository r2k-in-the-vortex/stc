//! CLI output: message formats, colors, lint levels, and verbose phase logs.
//!
//! Machine output goes to stdout (JSON diagnostics and reports); human and
//! short diagnostics and all `[stc]` logs go to stderr (CLI-and-Manifest §1).
//! Lint levels come from `[lints]` in `stc.toml`, overridden by `-D`/`-W`/`-A`
//! and `--deny warnings`; a warning promoted to an error keeps its original
//! code.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::time::Instant;

use stc_diag::{Diagnostic, Severity, render_human, render_json};
use stc_manifest::{LintLevel, LintSettings};
use stc_span::SourceMap;

/// The requested diagnostic format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    /// Multi-line human diagnostics with source snippets.
    Human,
    /// One JSON object per diagnostic or report.
    Json,
    /// One header line per diagnostic.
    Short,
}

impl Format {
    /// Returns `true` for JSON output.
    #[must_use]
    pub(crate) const fn is_json(self) -> bool {
        matches!(self, Self::Json)
    }
}

/// The `--color` choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ColorChoice {
    /// Color when stderr is a terminal.
    Auto,
    /// Always color.
    Always,
    /// Never color.
    Never,
}

impl ColorChoice {
    /// Parses a `--color` value.
    ///
    /// # Errors
    ///
    /// Rejects values other than `auto`, `always`, and `never`.
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "auto" => Ok(Self::Auto),
            "always" => Ok(Self::Always),
            "never" => Ok(Self::Never),
            other => Err(format!(
                "unsupported color choice `{other}` (expected `auto`, `always`, or `never`)"
            )),
        }
    }

    fn enabled(self) -> bool {
        match self {
            Self::Auto => std::io::stderr().is_terminal(),
            Self::Always => true,
            Self::Never => false,
        }
    }
}

/// CLI lint overrides in command-line order.
#[derive(Clone, Debug, Default)]
pub(crate) struct LintOverrides {
    /// Baseline level for warnings (`-D warnings` and friends).
    pub(crate) warnings: Option<LintLevel>,
    /// Per-code overrides.
    pub(crate) codes: BTreeMap<String, LintLevel>,
}

impl LintOverrides {
    /// Records one `-D`/`-W`/`-A` value.
    ///
    /// # Errors
    ///
    /// Rejects unknown codes and attempts to downgrade an error.
    pub(crate) fn set(&mut self, name: &str, level: LintLevel) -> Result<(), String> {
        if name == "warnings" {
            self.warnings = Some(level);
            return Ok(());
        }
        if !stc_manifest::is_known_lint_code(name) {
            return Err(format!("unknown lint code `{name}`"));
        }
        if name.starts_with("STC-E-") && level != LintLevel::Deny {
            return Err(format!(
                "lint `{name}` is an error and cannot be downgraded to `{}`",
                level.as_str()
            ));
        }
        self.codes.insert(name.to_owned(), level);
        Ok(())
    }
}

/// The effective lint levels.
#[derive(Clone, Debug)]
pub(crate) struct LintLevels {
    warnings: LintLevel,
    codes: BTreeMap<String, LintLevel>,
}

impl LintLevels {
    fn new(overrides: &LintOverrides) -> Self {
        Self {
            warnings: overrides.warnings.unwrap_or(LintLevel::Warn),
            codes: overrides.codes.clone(),
        }
    }

    fn merge_manifest(&mut self, lints: &LintSettings, overrides: &LintOverrides) {
        if overrides.warnings.is_none() {
            self.warnings = lints.warnings;
        }
        for (code, level) in &lints.codes {
            if !overrides.codes.contains_key(code) {
                self.codes.insert(code.clone(), *level);
            }
        }
    }

    fn level(&self, code: &str) -> LintLevel {
        self.codes.get(code).copied().unwrap_or(self.warnings)
    }
}

/// Renders diagnostics and reports with stable formats and lint filtering.
pub(crate) struct Reporter {
    format: Format,
    color: bool,
    quiet: bool,
    lints: LintLevels,
    overrides: LintOverrides,
    denied: bool,
}

impl Reporter {
    /// Creates a reporter from CLI options.
    #[must_use]
    pub(crate) fn new(
        format: Format,
        color: ColorChoice,
        quiet: bool,
        overrides: &LintOverrides,
    ) -> Self {
        Self {
            format,
            color: color.enabled(),
            quiet,
            lints: LintLevels::new(overrides),
            overrides: overrides.clone(),
            denied: false,
        }
    }

    /// Applies the project manifest's `[lints]` table below the CLI overrides.
    pub(crate) fn merge_manifest_lints(&mut self, lints: &LintSettings) {
        self.lints.merge_manifest(lints, &self.overrides);
    }

    /// Returns `true` when JSON output is selected.
    #[must_use]
    pub(crate) const fn is_json(&self) -> bool {
        self.format.is_json()
    }

    /// Returns `true` when a warning was promoted to an error.
    #[must_use]
    pub(crate) const fn denied(&self) -> bool {
        self.denied
    }

    /// Prints a non-diagnostic note unless quiet or writing JSON.
    pub(crate) fn note(&self, message: impl std::fmt::Display) {
        if !self.quiet && !self.format.is_json() {
            eprintln!("{message}");
        }
    }

    /// Reports one span-carrying diagnostic.
    pub(crate) fn diagnostic(&mut self, diagnostic: &Diagnostic, sources: &SourceMap) {
        let Some(severity) = self.severity(diagnostic.code, diagnostic.severity) else {
            return;
        };
        if self.quiet && severity == Severity::Warning {
            return;
        }
        match self.format {
            Format::Human => {
                let mut rendered = render_human(diagnostic, sources);
                if severity != diagnostic.severity {
                    rendered =
                        rendered.replacen(diagnostic.severity.as_str(), severity.as_str(), 1);
                }
                eprint!("{}", self.paint(&rendered, severity));
            }
            Format::Short => {
                let location = sources.location(diagnostic.primary);
                let path = sources.file_path(location.file).unwrap_or("<unknown>");
                let line = format!(
                    "{path}:{}:{}: {}[{}]: {}\n",
                    location.line, location.column, severity, diagnostic.code, diagnostic.message
                );
                eprint!("{}", self.paint(&line, severity));
            }
            Format::Json => {
                let rendered = render_json(diagnostic, sources);
                if severity == diagnostic.severity {
                    println!("{rendered}");
                } else {
                    println!(
                        "{}",
                        rendered.replacen(
                            &format!("\"severity\":\"{}\"", diagnostic.severity),
                            &format!("\"severity\":\"{severity}\""),
                            1
                        )
                    );
                }
            }
        }
    }

    /// Reports diagnostics with the map used to create them.
    pub(crate) fn diagnostics(&mut self, diagnostics: &[Diagnostic], sources: &SourceMap) {
        for diagnostic in diagnostics {
            self.diagnostic(diagnostic, sources);
        }
    }

    /// Reports a diagnostic without a source span (toolchain, build, or
    /// manifest-level messages).
    pub(crate) fn plain(&mut self, code: &str, severity: Severity, message: &str) {
        let Some(severity) = self.severity(code, severity) else {
            return;
        };
        if self.quiet && severity == Severity::Warning {
            return;
        }
        if self.format.is_json() {
            println!(
                "{{\"code\":\"{}\",\"severity\":\"{}\",\"message\":\"{}\"}}",
                escape_json(code),
                severity.as_str(),
                escape_json(message)
            );
        } else {
            let line = format!("{}[{code}]: {message}\n", severity.as_str());
            eprint!("{}", self.paint(&line, severity));
        }
    }

    fn severity(&mut self, code: &str, severity: Severity) -> Option<Severity> {
        if severity == Severity::Error {
            return Some(Severity::Error);
        }
        match self.lints.level(code) {
            LintLevel::Allow => None,
            LintLevel::Warn => Some(Severity::Warning),
            LintLevel::Deny => {
                self.denied = true;
                Some(Severity::Error)
            }
        }
    }

    fn paint(&self, text: &str, severity: Severity) -> String {
        if !self.color {
            return text.to_owned();
        }
        let color = match severity {
            Severity::Error => "\x1b[1;31m",
            Severity::Warning => "\x1b[1;33m",
        };
        let mut out = String::with_capacity(text.len() + 32);
        for line in text.split_inclusive('\n') {
            out.push_str(&paint_line(line, color));
        }
        out
    }
}

fn paint_line(line: &str, color: &str) -> String {
    const RESET: &str = "\x1b[0m";
    let mut out = String::with_capacity(line.len() + 16);
    let mut rest = line;
    if let Some(index) = rest.find("error[").or_else(|| rest.find("warning[")) {
        let after = &rest[index..];
        if let Some(close) = after.find(']') {
            out.push_str(&rest[..index]);
            out.push_str(color);
            out.push_str(&after[..=close]);
            out.push_str(RESET);
            rest = &after[close + 1..];
        }
    }
    let trimmed = rest.trim_start();
    if trimmed.starts_with('|')
        && let Some(start) = rest.find('^')
    {
        let end = rest[start..]
            .find(|character| character != '^')
            .map_or(rest.len(), |offset| start + offset);
        out.push_str(&rest[..start]);
        out.push_str(color);
        out.push_str(&rest[start..end]);
        out.push_str(RESET);
        out.push_str(&rest[end..]);
        return out;
    }
    out.push_str(rest);
    out
}

/// Verbose `[stc]` phase logging on stderr.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Log {
    quiet: bool,
    verbosity: u8,
}

impl Log {
    /// Creates a logger.
    #[must_use]
    pub(crate) const fn new(quiet: bool, verbosity: u8) -> Self {
        Self { quiet, verbosity }
    }

    /// Returns `true` when `-v` output is enabled.
    #[must_use]
    pub(crate) const fn verbose(self) -> bool {
        !self.quiet && self.verbosity >= 1
    }

    /// Returns `true` when `-vv` output is enabled.
    #[must_use]
    pub(crate) const fn trace(self) -> bool {
        !self.quiet && self.verbosity >= 2
    }

    /// Prints a phase line under `-v`.
    pub(crate) fn info(self, message: impl std::fmt::Display) {
        if self.verbose() {
            eprintln!("[stc] {message}");
        }
    }

    /// Prints a detail line under `-vv`.
    pub(crate) fn detail(self, message: impl std::fmt::Display) {
        if self.trace() {
            eprintln!("[stc] {message}");
        }
    }

    /// Prints an informational line unless `-q` was given.
    pub(crate) fn status(self, message: impl std::fmt::Display) {
        if !self.quiet {
            eprintln!("[stc] {message}");
        }
    }
}

/// Formats a phase duration for the verbose log.
#[must_use]
pub(crate) fn elapsed(start: Instant) -> String {
    format!("{:.1}ms", start.elapsed().as_secs_f64() * 1000.0)
}

/// Renders a resolved path for logs: no Windows verbatim prefix, and relative
/// to the working directory when it is below it. Paths are otherwise printed
/// as resolved.
#[must_use]
pub(crate) fn display_path(path: &std::path::Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let unbounded = std::path::Path::new(text);
    if let Ok(current) = std::env::current_dir() {
        let current = current.to_string_lossy();
        let current = current.strip_prefix(r"\\?\").unwrap_or(&current);
        if let Ok(relative) = unbounded.strip_prefix(current)
            && !relative.as_os_str().is_empty()
        {
            return relative.display().to_string().replace('\\', "/");
        }
    }
    text.replace('\\', "/")
}

fn escape_json(text: &str) -> String {
    let mut out = String::new();
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            character if (character as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04X}", character as u32));
            }
            character => out.push(character),
        }
    }
    out
}
