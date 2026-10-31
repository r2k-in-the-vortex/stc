//! Built-in test suite discovery, process-isolated runner, and reporting
//! (Testing and Verification §2, M6 phase 6).
//!
//! `stc test` discovers `{stc:test}` POUs in the current package, builds one
//! artifact per test through the driver, and runs each artifact in its own test
//! host process. This crate owns the syntax-level discovery and manifest
//! overrides, the child-process runner (timeout and result parsing), the raw
//! coverage sidecar parser, and the stable human/JSON report shapes. It does not
//! compile or execute modules.

#![forbid(unsafe_code)]

pub mod coverage;
pub mod discovery;
pub mod report;
pub mod runner;

pub use coverage::{RawCoverage, RawCoverageError, parse as parse_raw_coverage};

pub use discovery::{
    Discovery, SourceFileRef, TestCase, TestKind, discover, filter_tests, wrapper_source,
};
pub use report::{human_failure_lines, human_result_line, json_result, summary_line};
pub use runner::{Assertion, TestOutcome, TestStatus, run_process};
