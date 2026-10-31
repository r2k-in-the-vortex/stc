//! Raw coverage sidecar parsing (WL-10 §6.2, C-5).
//!
//! The test host writes one raw sidecar per test in the internal
//! `stc-coverage-raw/1` protocol. The driver reads it after the process exits
//! and aggregates the nonzero counters into canonical source sites. This module
//! validates that protocol; it is file-independent and usable without a
//! running host.
//!
//! Malformed data is an internal protocol error, not a compiler diagnostic: the
//! driver degrades it to a coverage warning and excludes the test from the
//! aggregate.

use std::fmt;

use stc_got::coverage::{MAX_SITES, RAW_PROTOCOL};

/// One test's raw coverage counters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCoverage {
    /// Number of coverage sites declared by the test artifact.
    pub sites: u32,
    /// Nonzero counters in strictly ascending site-ID order.
    pub counts: Vec<(u32, u64)>,
}

/// A malformed raw coverage sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCoverageError {
    line: usize,
    message: String,
}

impl RawCoverageError {
    fn new(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }

    /// Returns the 1-based line number, or `0` for a file-level error.
    #[must_use]
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the human-readable error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for RawCoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            formatter.write_str(&self.message)
        } else {
            write!(formatter, "line {}: {}", self.line, self.message)
        }
    }
}

impl std::error::Error for RawCoverageError {}

/// Parses the raw coverage sidecar text.
///
/// The first line is `stc-coverage-raw/1 <sites>`; every following non-blank
/// line is `<id> <count>` for a nonzero counter. Site IDs must be strictly
/// ascending and less than `<sites>`, and counts must fit in `u64`. A
/// header-only file is valid; one trailing blank line is ignored.
pub fn parse(text: &str) -> Result<RawCoverage, RawCoverageError> {
    let mut lines = text.lines();
    let Some(header) = lines.next() else {
        return Err(RawCoverageError::new(
            0,
            format!("missing header: expected `{RAW_PROTOCOL} <sites>`"),
        ));
    };
    let mut fields = header.split_whitespace();
    let Some(protocol) = fields.next() else {
        return Err(RawCoverageError::new(
            0,
            format!("missing header: expected `{RAW_PROTOCOL} <sites>`"),
        ));
    };
    if protocol != RAW_PROTOCOL {
        return Err(RawCoverageError::new(
            1,
            format!("expected protocol `{RAW_PROTOCOL}`, found `{protocol}`"),
        ));
    }
    let Some(sites_text) = fields.next() else {
        return Err(RawCoverageError::new(
            1,
            format!("missing site count: expected `{RAW_PROTOCOL} <sites>`"),
        ));
    };
    if fields.next().is_some() {
        return Err(RawCoverageError::new(
            1,
            "unexpected field in the coverage header",
        ));
    }
    let sites = sites_text
        .parse::<u32>()
        .map_err(|_| RawCoverageError::new(1, format!("invalid site count `{sites_text}`")))?;
    if sites > MAX_SITES {
        return Err(RawCoverageError::new(
            1,
            format!("site count {sites} exceeds the maximum of {MAX_SITES}"),
        ));
    }

    let mut body: Vec<&str> = lines.collect();
    if body.last().is_some_and(|line| line.trim().is_empty()) {
        body.pop();
    }
    let mut counts = Vec::new();
    let mut previous = None;
    for (index, line) in body.iter().enumerate() {
        let line_number = index + 2;
        if line.trim().is_empty() {
            return Err(RawCoverageError::new(
                line_number,
                "blank line in counter data",
            ));
        }
        let mut fields = line.split_whitespace();
        let Some(id_text) = fields.next() else {
            return Err(RawCoverageError::new(line_number, "missing counter ID"));
        };
        let Some(count_text) = fields.next() else {
            return Err(RawCoverageError::new(line_number, "missing hit count"));
        };
        if fields.next().is_some() {
            return Err(RawCoverageError::new(
                line_number,
                "unexpected field in counter data",
            ));
        }
        let id = id_text.parse::<u32>().map_err(|_| {
            RawCoverageError::new(line_number, format!("invalid site ID `{id_text}`"))
        })?;
        let count = count_text.parse::<u64>().map_err(|_| {
            RawCoverageError::new(line_number, format!("invalid hit count `{count_text}`"))
        })?;
        if id >= sites {
            return Err(RawCoverageError::new(
                line_number,
                format!("site ID {id} is out of range for {sites} sites"),
            ));
        }
        if let Some(previous) = previous
            && id <= previous
        {
            return Err(RawCoverageError::new(
                line_number,
                format!("site ID {id} is not strictly ascending after {previous}"),
            ));
        }
        previous = Some(id);
        counts.push((id, count));
    }
    Ok(RawCoverage { sites, counts })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(text: &str) -> RawCoverageError {
        parse(text).expect_err("malformed raw coverage")
    }

    #[test]
    fn parses_nonzero_counters() {
        let parsed = parse("stc-coverage-raw/1 5\n0 2\n4 1\n").expect("valid sidecar");
        assert_eq!(parsed.sites, 5);
        assert_eq!(parsed.counts, vec![(0, 2), (4, 1)]);
    }

    #[test]
    fn accepts_header_only_files() {
        for text in [
            "stc-coverage-raw/1 0\n",
            "stc-coverage-raw/1 0",
            "stc-coverage-raw/1 3\n",
        ] {
            let parsed = parse(text).expect("header-only sidecar");
            assert!(parsed.counts.is_empty(), "no counters in {text:?}");
        }
        assert_eq!(
            parse("stc-coverage-raw/1 3\n").expect("header-only").sites,
            3
        );
    }

    #[test]
    fn rejects_a_wrong_protocol() {
        let error = error("stc-coverage-raw/2 0\n");
        assert_eq!(error.line(), 1);
        assert!(error.message().contains("protocol"), "{error}");
    }

    #[test]
    fn rejects_missing_or_short_headers() {
        for text in [
            "",
            "\n",
            "stc-coverage-raw/1",
            "stc-coverage-raw/1 \n",
            "stc-coverage-raw/1 nope\n",
            "stc-coverage-raw/1 0 extra\n",
        ] {
            let error = error(text);
            assert!(error.line() <= 1, "{text:?}: {error}");
        }
    }

    #[test]
    fn rejects_site_counts_above_the_cap() {
        let error = error("stc-coverage-raw/1 1000001\n");
        assert_eq!(error.line(), 1);
        assert!(error.message().contains("1000001"), "{error}");
        let parsed = parse("stc-coverage-raw/1 1000000\n").expect("the cap is inclusive");
        assert_eq!(parsed.sites, MAX_SITES);
    }

    #[test]
    fn rejects_ids_out_of_range() {
        let error = error("stc-coverage-raw/1 2\n2 1\n");
        assert_eq!(error.line(), 2);
        assert!(error.message().contains("out of range"), "{error}");
    }

    #[test]
    fn rejects_duplicate_or_unsorted_ids() {
        for text in [
            "stc-coverage-raw/1 3\n1 1\n1 2\n",
            "stc-coverage-raw/1 3\n2 1\n1 2\n",
        ] {
            let error = error(text);
            assert_eq!(error.line(), 3);
            assert!(error.message().contains("ascending"), "{error}");
        }
    }

    #[test]
    fn rejects_non_numeric_fields() {
        let cases = [
            ("stc-coverage-raw/1 3\nx 1\n", "site ID"),
            ("stc-coverage-raw/1 3\n0 x\n", "hit count"),
            ("stc-coverage-raw/1 3\n0 -1\n", "hit count"),
            (
                "stc-coverage-raw/1 3\n0 18446744073709551616\n",
                "hit count",
            ),
            ("stc-coverage-raw/1 3\n0 1 extra\n", "unexpected field"),
        ];
        for (text, expected) in cases {
            let error = error(text);
            assert_eq!(error.line(), 2, "{text:?}");
            assert!(error.message().contains(expected), "{text:?}: {error}");
        }
    }

    #[test]
    fn handles_trailing_blank_lines() {
        let parsed = parse("stc-coverage-raw/1 2\n0 1\n\n").expect("one trailing blank line");
        assert_eq!(parsed.counts, vec![(0, 1)]);

        for text in [
            "stc-coverage-raw/1 2\n\n0 1\n",
            "stc-coverage-raw/1 2\n0 1\n\n\n",
        ] {
            let error = error(text);
            assert!(error.message().contains("blank"), "{text:?}: {error}");
        }
    }
}
