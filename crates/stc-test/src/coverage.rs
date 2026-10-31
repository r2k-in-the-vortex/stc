//! Raw coverage sidecar parsing and statement-coverage reporting (WL-10 §6.2,
//! §6.3, §7).
//!
//! The test host writes one raw sidecar per test in the internal
//! `stc-coverage-raw/1` protocol. The driver reads it after the process exits
//! and aggregates the nonzero counters into canonical source sites. This module
//! validates that protocol, aggregates per-test data across a suite, and
//! renders the native JSON, LCOV, and human reports; it is file-independent and
//! usable without a running host.
//!
//! Malformed data is an internal protocol error, not a compiler diagnostic: the
//! driver degrades it to a coverage warning and excludes the test from the
//! aggregate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde_json::{Map, Value, json};
use stc_got::coverage::{MAX_SITES, RAW_PROTOCOL};

use crate::runner::TestStatus;

/// The native coverage report protocol string (`coverage.json`).
pub const REPORT_PROTOCOL: &str = "stc-coverage/1";

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

/// What one coverage site measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoverageKind {
    /// One executable statement entry (WL-10 C-8).
    Statement,
}

impl CoverageKind {
    /// Returns the report spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Statement => "statement",
        }
    }
}

/// One canonical coverage site, mapped onto a reportable source file.
///
/// `start` and `end` are byte offsets in the source file's `SourceMap`; the
/// canonical identity is `(file, start, end, kind)` (WL-10 C-6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoveragePoint {
    /// Package-relative report path with forward slashes.
    pub file: String,
    /// Start byte offset of the statement.
    pub start: u32,
    /// End byte offset of the statement.
    pub end: u32,
    /// 1-based line of the statement start.
    pub line: u32,
    /// 1-based column (Unicode scalar values) of the statement start.
    pub column: u32,
    /// What the site measures.
    pub kind: CoverageKind,
}

impl CoveragePoint {
    fn key(&self) -> CanonicalKey {
        (self.file.clone(), self.start, self.end, self.kind)
    }
}

type CanonicalKey = (String, u32, u32, CoverageKind);

/// How one test's coverage data arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CoverageData {
    /// Parsed nonzero counters from the raw sidecar.
    Reported {
        /// Nonzero `(site id, hit count)` pairs in ascending ID order.
        counts: Vec<(u32, u64)>,
    },
    /// The raw sidecar was missing, malformed, or inconsistent and the test is
    /// excluded from the aggregate (`STC-W-COVERAGE-001`).
    Missing,
    /// The test timed out before the host could write a sidecar.
    Timeout,
}

/// One test's coverage contribution.
///
/// `sites` maps the test artifact's local site ID to its canonical source
/// point; `None` marks a site omitted from reporting because it cannot be
/// attributed to a reportable file (`STC-W-COVERAGE-002`). The table is kept
/// even for tests without coverage data so the aggregate site universe includes
/// every instrumented site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageTest {
    /// Test POU name, exactly as reported by the host.
    pub name: String,
    /// Final test status.
    pub status: TestStatus,
    /// Local site-ID to canonical point mapping, in artifact ID order.
    pub sites: Vec<Option<CoveragePoint>>,
    /// How the test's counters arrived.
    pub data: CoverageData,
}

/// A whole coverage run: every selected test and its instrumented sites.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageRun {
    /// Root package name.
    pub package: String,
    /// Active test name filters, in command-line order.
    pub filters: Vec<String>,
    /// Tests in run order.
    pub tests: Vec<CoverageTest>,
}

/// One test's coverage state in the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoverageState {
    /// Counters were reported and contribute to the aggregate.
    Reported,
    /// Data was missing, malformed, or inconsistent (`STC-W-COVERAGE-001`).
    Missing,
    /// The test timed out; no data was expected.
    Timeout,
}

impl CoverageState {
    /// Returns the report spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reported => "reported",
            Self::Missing => "missing",
            Self::Timeout => "timeout",
        }
    }
}

/// One canonical site and its summed hit count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SiteCoverage {
    /// The canonical site.
    pub point: CoveragePoint,
    /// Summed counts across contributing tests; zero when never hit.
    pub count: u64,
}

/// Per-file statement and line totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileCoverage {
    /// Package-relative report path.
    pub file: String,
    /// Number of canonical sites in the file.
    pub sites: usize,
    /// Sites hit by at least one test.
    pub covered: usize,
    /// Distinct lines holding at least one site.
    pub lines: usize,
    /// Lines with a nonzero summed count.
    pub lines_covered: usize,
}

/// Suite-wide statement and line totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CoverageTotals {
    /// Number of canonical sites.
    pub sites: usize,
    /// Sites hit by at least one test.
    pub covered: usize,
    /// Distinct lines holding at least one site.
    pub lines: usize,
    /// Lines with a nonzero summed count.
    pub lines_covered: usize,
}

/// One test's entry in the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestCoverage {
    /// Test POU name.
    pub name: String,
    /// Final test status.
    pub status: TestStatus,
    /// Coverage state.
    pub state: CoverageState,
    /// Ascending canonical site indices hit by this test; empty unless
    /// `state` is [`CoverageState::Reported`].
    pub covered: Vec<usize>,
}

/// The aggregated, render-ready coverage report (WL-10 §7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageReport {
    /// Root package name.
    pub package: String,
    /// Active test name filters.
    pub filters: Vec<String>,
    /// Canonical sites, sorted by `(file, start, end)`.
    pub sites: Vec<SiteCoverage>,
    /// Per-file totals, sorted by path.
    pub files: Vec<FileCoverage>,
    /// Suite-wide totals.
    pub totals: CoverageTotals,
    /// Tests in run order.
    pub tests: Vec<TestCoverage>,
}

/// Aggregates per-test coverage into the canonical report (WL-10 §6.3, §7).
///
/// The site universe is the union of every instrumented artifact's site table,
/// including tests whose data is missing or which timed out. Counts are summed
/// with saturation across tests that reported data; a site is covered when its
/// summed count is nonzero. Sites omitted with `STC-W-COVERAGE-002` do not
/// enter the universe.
#[must_use]
pub fn aggregate(run: &CoverageRun) -> CoverageReport {
    let mut points: BTreeMap<CanonicalKey, CoveragePoint> = BTreeMap::new();
    let mut counts: BTreeMap<CanonicalKey, u64> = BTreeMap::new();
    let mut hits: Vec<BTreeSet<CanonicalKey>> = Vec::with_capacity(run.tests.len());
    let mut states: Vec<CoverageState> = Vec::with_capacity(run.tests.len());

    for test in &run.tests {
        for point in test.sites.iter().flatten() {
            points.entry(point.key()).or_insert_with(|| point.clone());
        }
        let state = match test.status {
            TestStatus::Timeout => CoverageState::Timeout,
            _ => match test.data {
                CoverageData::Reported { .. } => CoverageState::Reported,
                CoverageData::Missing | CoverageData::Timeout => CoverageState::Missing,
            },
        };
        let mut test_hits = BTreeSet::new();
        if state == CoverageState::Reported
            && let CoverageData::Reported { counts: local } = &test.data
        {
            for &(id, count) in local {
                let Some(Some(point)) = test.sites.get(id as usize) else {
                    continue;
                };
                let key = point.key();
                counts
                    .entry(key.clone())
                    .and_modify(|total| *total = total.saturating_add(count))
                    .or_insert(count);
                if count > 0 {
                    test_hits.insert(key);
                }
            }
        }
        hits.push(test_hits);
        states.push(state);
    }

    let keys: Vec<CanonicalKey> = points.keys().cloned().collect();
    let index: BTreeMap<CanonicalKey, usize> = keys
        .iter()
        .cloned()
        .enumerate()
        .map(|(position, key)| (key, position))
        .collect();
    let sites: Vec<SiteCoverage> = keys
        .iter()
        .map(|key| SiteCoverage {
            point: points[key].clone(),
            count: counts.get(key).copied().unwrap_or(0),
        })
        .collect();

    let mut per_file: BTreeMap<String, FileStats> = BTreeMap::new();
    for site in &sites {
        let stats = per_file.entry(site.point.file.clone()).or_default();
        stats.sites += 1;
        if site.count > 0 {
            stats.covered += 1;
        }
        let line = stats.lines.entry(site.point.line).or_insert(0);
        *line = line.saturating_add(site.count);
    }
    let files: Vec<FileCoverage> = per_file
        .into_iter()
        .map(|(file, stats)| FileCoverage {
            file,
            sites: stats.sites,
            covered: stats.covered,
            lines: stats.lines.len(),
            lines_covered: stats.lines.values().filter(|count| **count > 0).count(),
        })
        .collect();
    let totals = CoverageTotals {
        sites: sites.len(),
        covered: sites.iter().filter(|site| site.count > 0).count(),
        lines: files.iter().map(|file| file.lines).sum(),
        lines_covered: files.iter().map(|file| file.lines_covered).sum(),
    };
    let tests: Vec<TestCoverage> = run
        .tests
        .iter()
        .enumerate()
        .map(|(position, test)| TestCoverage {
            name: test.name.clone(),
            status: test.status,
            state: states[position],
            covered: if states[position] == CoverageState::Reported {
                hits[position]
                    .iter()
                    .filter_map(|key| index.get(key).copied())
                    .collect()
            } else {
                Vec::new()
            },
        })
        .collect();

    CoverageReport {
        package: run.package.clone(),
        filters: run.filters.clone(),
        sites,
        files,
        totals,
        tests,
    }
}

#[derive(Default)]
struct FileStats {
    sites: usize,
    covered: usize,
    lines: BTreeMap<u32, u64>,
}

/// Returns a percentage with one decimal place, rounded half away from zero
/// (WL-10 C-9), or `None` when `total` is zero.
///
/// Uses integer arithmetic so every host renders identical bytes.
#[must_use]
pub fn percent(covered: usize, total: usize) -> Option<f64> {
    if total == 0 {
        return None;
    }
    let covered = covered as u128;
    let total = total as u128;
    let tenths = covered.saturating_mul(2000).saturating_add(total) / total.saturating_mul(2);
    Some(tenths as f64 / 10.0)
}

fn percent_json(covered: usize, total: usize) -> Value {
    percent(covered, total).map_or(Value::Null, Value::from)
}

fn format_percent(covered: usize, total: usize) -> String {
    percent(covered, total).map_or_else(|| "0.0".to_owned(), |value| format!("{value:.1}"))
}

/// Renders the pretty-printed native JSON report (`coverage.json`).
///
/// Top-level and nested keys serialize in sorted order, sites sort by canonical
/// key, files sort by path, and tests stay in run order. No timestamps, build
/// identifiers, host data, or absolute paths appear.
#[must_use]
pub fn render_json(report: &CoverageReport) -> String {
    let totals = &report.totals;
    let files: Vec<Value> = report
        .files
        .iter()
        .map(|file| {
            json!({
                "file": file.file,
                "sites": file.sites,
                "covered": file.covered,
                "percent": percent_json(file.covered, file.sites),
                "lines": file.lines,
                "lines_covered": file.lines_covered,
                "line_percent": percent_json(file.lines_covered, file.lines),
            })
        })
        .collect();
    let sites: Vec<Value> = report
        .sites
        .iter()
        .map(|site| {
            json!({
                "file": site.point.file,
                "line": site.point.line,
                "column": site.point.column,
                "count": site.count,
            })
        })
        .collect();
    let tests: Vec<Value> = report
        .tests
        .iter()
        .map(|test| {
            let mut object = Map::new();
            object.insert("name".to_owned(), Value::String(test.name.clone()));
            object.insert(
                "status".to_owned(),
                Value::String(test.status.as_str().to_owned()),
            );
            object.insert(
                "coverage".to_owned(),
                Value::String(test.state.as_str().to_owned()),
            );
            if test.state == CoverageState::Reported {
                object.insert("sites_covered".to_owned(), Value::from(test.covered.len()));
                object.insert(
                    "covered".to_owned(),
                    Value::Array(
                        test.covered
                            .iter()
                            .map(|index| Value::from(*index))
                            .collect(),
                    ),
                );
            }
            Value::Object(object)
        })
        .collect();
    let value = json!({
        "protocol": REPORT_PROTOCOL,
        "profile": stc_manifest::PROFILE_ID,
        "package": report.package,
        "kind": CoverageKind::Statement.as_str(),
        "filters": report.filters,
        "totals": {
            "sites": totals.sites,
            "covered": totals.covered,
            "percent": percent_json(totals.covered, totals.sites),
            "lines": totals.lines,
            "lines_covered": totals.lines_covered,
            "line_percent": percent_json(totals.lines_covered, totals.lines),
        },
        "files": files,
        "sites": sites,
        "tests": tests,
    });
    let mut text =
        serde_json::to_string_pretty(&value).expect("coverage report is a valid JSON value");
    text.push('\n');
    text
}

/// Renders the LCOV tracefile (`lcov.info`).
///
/// One record per file, sorted by path. `DA:` lines carry one entry per start
/// line with the summed counts of every site starting on that line; `LF:` and
/// `LH:` count lines. Zero sites render as an empty string.
#[must_use]
pub fn render_lcov(report: &CoverageReport) -> String {
    let mut records: BTreeMap<&str, BTreeMap<u32, u64>> = BTreeMap::new();
    for site in &report.sites {
        let lines = records.entry(site.point.file.as_str()).or_default();
        let count = lines.entry(site.point.line).or_insert(0);
        *count = count.saturating_add(site.count);
    }
    let mut text = String::new();
    for (file, lines) in records {
        text.push_str("TN:\n");
        text.push_str("SF:");
        text.push_str(file);
        text.push('\n');
        for (line, count) in &lines {
            text.push_str(&format!("DA:{line},{count}\n"));
        }
        text.push_str(&format!("LF:{}\n", lines.len()));
        let hit = lines.values().filter(|count| **count > 0).count();
        text.push_str(&format!("LH:{hit}\n"));
        text.push_str("end_of_record\n");
    }
    text
}

/// Renders the human coverage summary.
///
/// The first line reports suite-wide statement and line coverage; one indented
/// line per file follows, sorted by path. A run with no sites renders
/// `coverage: no sites`.
#[must_use]
pub fn render_human(report: &CoverageReport) -> String {
    let totals = &report.totals;
    if totals.sites == 0 {
        return "coverage: no sites\n".to_owned();
    }
    let mut text = format!(
        "coverage: {}/{} statements ({}%), {}/{} lines ({}%)\n",
        totals.covered,
        totals.sites,
        format_percent(totals.covered, totals.sites),
        totals.lines_covered,
        totals.lines,
        format_percent(totals.lines_covered, totals.lines),
    );
    for file in &report.files {
        text.push_str(&format!(
            "  {}: {}/{} statements ({}%), {}/{} lines ({}%)\n",
            file.file,
            file.covered,
            file.sites,
            format_percent(file.covered, file.sites),
            file.lines_covered,
            file.lines,
            format_percent(file.lines_covered, file.lines),
        ));
    }
    text
}

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

#[cfg(test)]
mod report_tests {
    use super::*;

    fn point(file: &str, start: u32, end: u32, line: u32, column: u32) -> CoveragePoint {
        CoveragePoint {
            file: file.to_owned(),
            start,
            end,
            line,
            column,
            kind: CoverageKind::Statement,
        }
    }

    fn test(
        name: &str,
        status: TestStatus,
        sites: Vec<Option<CoveragePoint>>,
        counts: Vec<(u32, u64)>,
    ) -> CoverageTest {
        CoverageTest {
            name: name.to_owned(),
            status,
            sites,
            data: CoverageData::Reported { counts },
        }
    }

    fn run(package: &str, tests: Vec<CoverageTest>) -> CoverageRun {
        CoverageRun {
            package: package.to_owned(),
            filters: Vec::new(),
            tests,
        }
    }

    fn json(report: &CoverageReport) -> Value {
        serde_json::from_str(&render_json(report)).expect("report JSON parses")
    }

    #[test]
    fn aggregate_unions_sites_and_sums_counts() {
        let a = point("src/a.st", 10, 20, 2, 3);
        let b = point("src/b.st", 30, 40, 5, 1);
        let report = aggregate(&run(
            "demo",
            vec![
                test(
                    "One",
                    TestStatus::Pass,
                    vec![Some(a.clone()), Some(b.clone())],
                    vec![(0, 2), (1, 1)],
                ),
                test(
                    "Two",
                    TestStatus::Pass,
                    vec![Some(a), Some(b)],
                    vec![(0, 3)],
                ),
            ],
        ));
        assert_eq!(report.package, "demo");
        assert_eq!(report.sites.len(), 2);
        assert_eq!(report.sites[0].point.file, "src/a.st");
        assert_eq!(report.sites[0].count, 5);
        assert_eq!(report.sites[1].count, 1);
        assert_eq!(report.totals.sites, 2);
        assert_eq!(report.totals.covered, 2);
        assert_eq!(report.tests[0].covered, vec![0, 1]);
        assert_eq!(report.tests[1].covered, vec![0]);
        assert_eq!(report.tests[0].state, CoverageState::Reported);
    }

    #[test]
    fn aggregate_handles_disjoint_tests() {
        let a = point("src/a.st", 0, 5, 1, 1);
        let b = point("src/b.st", 0, 5, 1, 1);
        let report = aggregate(&run(
            "demo",
            vec![
                test("One", TestStatus::Pass, vec![Some(a)], vec![(0, 1)]),
                test("Two", TestStatus::Pass, vec![Some(b)], vec![(0, 1)]),
            ],
        ));
        assert_eq!(report.sites.len(), 2);
        assert_eq!(report.tests[0].covered, vec![0]);
        assert_eq!(report.tests[1].covered, vec![1]);
        assert_eq!(report.totals.covered, 2);
        assert!(report.files.iter().all(|file| file.covered == 1));
    }

    #[test]
    fn aggregate_projects_sites_onto_lines() {
        let first = point("src/a.st", 0, 5, 3, 1);
        let second = point("src/a.st", 10, 15, 3, 7);
        let report = aggregate(&run(
            "demo",
            vec![test(
                "One",
                TestStatus::Pass,
                vec![Some(first), Some(second)],
                vec![(1, 2)],
            )],
        ));
        assert_eq!(report.sites.len(), 2);
        assert_eq!(report.files.len(), 1);
        let file = &report.files[0];
        assert_eq!(file.sites, 2);
        assert_eq!(file.covered, 1);
        assert_eq!(file.lines, 1);
        assert_eq!(file.lines_covered, 1);
        assert_eq!(report.totals.lines, 1);
        assert_eq!(report.totals.lines_covered, 1);
        assert_eq!(
            render_lcov(&report),
            "TN:\nSF:src/a.st\nDA:3,2\nLF:1\nLH:1\nend_of_record\n"
        );
    }

    #[test]
    fn aggregate_handles_zero_sites() {
        let report = aggregate(&CoverageRun {
            package: "demo".to_owned(),
            filters: vec!["stats".to_owned()],
            tests: Vec::new(),
        });
        assert!(report.sites.is_empty());
        assert!(report.files.is_empty());
        assert_eq!(report.totals, CoverageTotals::default());
        assert_eq!(render_human(&report), "coverage: no sites\n");
        assert_eq!(render_lcov(&report), "");
        let value = json(&report);
        assert_eq!(value["protocol"], "stc-coverage/1");
        assert_eq!(value["package"], "demo");
        assert_eq!(value["filters"][0], "stats");
        assert_eq!(value["totals"]["percent"], Value::Null);
        assert_eq!(value["totals"]["line_percent"], Value::Null);
        assert_eq!(value["files"], json!([]));
        assert_eq!(value["sites"], json!([]));
        assert_eq!(value["tests"], json!([]));
    }

    #[test]
    fn percent_rounds_half_up_with_one_decimal() {
        assert_eq!(percent(1, 3), Some(33.3));
        assert_eq!(percent(2, 3), Some(66.7));
        assert_eq!(percent(1, 2), Some(50.0));
        assert_eq!(percent(3, 5), Some(60.0));
        assert_eq!(percent(0, 5), Some(0.0));
        assert_eq!(percent(5, 5), Some(100.0));
        assert_eq!(percent(1, 2000), Some(0.1));
        assert_eq!(percent(1, 2001), Some(0.0));
        assert_eq!(percent(0, 0), None);
    }

    #[test]
    fn aggregate_saturates_summed_counts() {
        let a = point("src/a.st", 0, 5, 1, 1);
        let report = aggregate(&run(
            "demo",
            vec![
                test(
                    "One",
                    TestStatus::Pass,
                    vec![Some(a.clone())],
                    vec![(0, u64::MAX)],
                ),
                test("Two", TestStatus::Pass, vec![Some(a)], vec![(0, 2)]),
            ],
        ));
        assert_eq!(report.sites[0].count, u64::MAX);
        assert_eq!(report.totals.covered, 1);
    }

    #[test]
    fn aggregate_ignores_reported_counts_for_timeouts() {
        let a = point("src/a.st", 0, 5, 1, 1);
        let report = aggregate(&run(
            "demo",
            vec![CoverageTest {
                name: "Slow".to_owned(),
                status: TestStatus::Timeout,
                sites: vec![Some(a)],
                data: CoverageData::Reported {
                    counts: vec![(0, 9)],
                },
            }],
        ));
        assert_eq!(report.tests[0].state, CoverageState::Timeout);
        assert!(report.tests[0].covered.is_empty());
        assert_eq!(report.sites.len(), 1);
        assert_eq!(report.sites[0].count, 0);
    }

    #[test]
    fn aggregate_marks_missing_and_timeout_tests() {
        let a = point("src/a.st", 0, 5, 1, 1);
        let report = aggregate(&run(
            "demo",
            vec![
                CoverageTest {
                    name: "Missing".to_owned(),
                    status: TestStatus::Pass,
                    sites: vec![Some(a.clone())],
                    data: CoverageData::Missing,
                },
                CoverageTest {
                    name: "Timed".to_owned(),
                    status: TestStatus::Timeout,
                    sites: vec![Some(a)],
                    data: CoverageData::Timeout,
                },
            ],
        ));
        assert_eq!(report.tests[0].state, CoverageState::Missing);
        assert!(report.tests[0].covered.is_empty());
        assert_eq!(report.tests[1].state, CoverageState::Timeout);
        assert!(report.tests[1].covered.is_empty());
        assert_eq!(report.sites.len(), 1);
        assert_eq!(report.sites[0].count, 0);
        assert_eq!(report.totals.covered, 0);

        let value = json(&report);
        assert_eq!(value["tests"][0]["coverage"], "missing");
        assert!(value["tests"][0].get("covered").is_none());
        assert!(value["tests"][0].get("sites_covered").is_none());
        assert_eq!(value["tests"][1]["coverage"], "timeout");
        assert_eq!(value["tests"][1]["status"], "timeout");
    }

    #[test]
    fn aggregate_is_independent_of_local_site_order() {
        let a = point("src/a.st", 10, 20, 2, 1);
        let b = point("src/b.st", 5, 9, 1, 1);
        let first = aggregate(&run(
            "demo",
            vec![test(
                "One",
                TestStatus::Pass,
                vec![Some(a.clone()), Some(b.clone())],
                vec![(0, 1), (1, 2)],
            )],
        ));
        let second = aggregate(&run(
            "demo",
            vec![test(
                "One",
                TestStatus::Pass,
                vec![Some(b), Some(a)],
                vec![(0, 2), (1, 1)],
            )],
        ));
        assert_eq!(first.sites, second.sites);
        assert_eq!(first.files, second.files);
        assert_eq!(first.totals, second.totals);
        assert_eq!(first.tests[0].covered, second.tests[0].covered);
        assert_eq!(render_json(&first), render_json(&second));
    }

    #[test]
    fn render_json_uses_sorted_keys_and_documented_shape() {
        let a = point("src/a.st", 10, 20, 2, 3);
        let report = aggregate(&run(
            "demo",
            vec![test("One", TestStatus::Pass, vec![Some(a)], vec![(0, 1)])],
        ));
        let text = render_json(&report);
        assert!(text.ends_with("}\n"));
        assert!(text.find("\"files\"").unwrap() < text.find("\"filters\"").unwrap());
        assert!(text.find("\"sites\"").unwrap() < text.find("\"tests\"").unwrap());
        assert!(text.find("\"tests\"").unwrap() < text.find("\"totals\"").unwrap());
        let value: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["protocol"], REPORT_PROTOCOL);
        assert_eq!(value["profile"], stc_manifest::PROFILE_ID);
        assert_eq!(value["kind"], "statement");
        assert_eq!(value["totals"]["sites"], 1);
        assert_eq!(value["totals"]["covered"], 1);
        assert_eq!(value["totals"]["percent"], 100.0);
        assert_eq!(value["files"][0]["percent"], 100.0);
        assert_eq!(value["files"][0]["line_percent"], 100.0);
        assert_eq!(value["sites"][0]["file"], "src/a.st");
        assert_eq!(value["sites"][0]["line"], 2);
        assert_eq!(value["sites"][0]["column"], 3);
        assert_eq!(value["sites"][0]["count"], 1);
        assert_eq!(value["tests"][0]["name"], "One");
        assert_eq!(value["tests"][0]["status"], "pass");
        assert_eq!(value["tests"][0]["coverage"], "reported");
        assert_eq!(value["tests"][0]["sites_covered"], 1);
        assert_eq!(value["tests"][0]["covered"], json!([0]));
    }

    #[test]
    fn render_human_reports_totals_then_files() {
        let a = point("src/b.st", 0, 5, 1, 1);
        let b = point("src/a.st", 0, 5, 1, 1);
        let report = aggregate(&run(
            "demo",
            vec![test(
                "One",
                TestStatus::Pass,
                vec![Some(a), Some(b)],
                vec![(0, 1)],
            )],
        ));
        assert_eq!(
            render_human(&report),
            "coverage: 1/2 statements (50.0%), 1/2 lines (50.0%)\n  src/a.st: 0/1 statements (0.0%), 0/1 lines (0.0%)\n  src/b.st: 1/1 statements (100.0%), 1/1 lines (100.0%)\n"
        );
    }

    #[test]
    fn render_lcov_sorts_files_and_sums_duplicate_lines() {
        let b = point("src/b.st", 0, 5, 4, 1);
        let a1 = point("src/a.st", 0, 5, 7, 1);
        let a2 = point("src/a.st", 10, 15, 7, 9);
        let report = aggregate(&run(
            "demo",
            vec![test(
                "One",
                TestStatus::Pass,
                vec![Some(b), Some(a2), Some(a1)],
                vec![(0, 1), (1, 2), (2, 3)],
            )],
        ));
        assert_eq!(
            render_lcov(&report),
            "TN:\nSF:src/a.st\nDA:7,5\nLF:1\nLH:1\nend_of_record\nTN:\nSF:src/b.st\nDA:4,1\nLF:1\nLH:1\nend_of_record\n"
        );
    }

    #[test]
    fn renderers_are_byte_stable() {
        let a = point("src/a.st", 0, 5, 1, 1);
        let run = run(
            "demo",
            vec![test("One", TestStatus::Pass, vec![Some(a)], vec![(0, 1)])],
        );
        let first = aggregate(&run);
        let second = aggregate(&run.clone());
        assert_eq!(render_json(&first), render_json(&second));
        assert_eq!(render_lcov(&first), render_lcov(&second));
        assert_eq!(render_human(&first), render_human(&second));
        let text = render_json(&first);
        assert!(!text.contains("2026"), "no timestamps");
        assert!(!text.contains("C:\\"), "no absolute paths");
        assert!(!text.contains("duration"), "no durations");
    }
}
