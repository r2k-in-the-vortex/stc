//! Smoke test for the committed `fuzz_manifest` seeds.
//!
//! The fuzz target itself needs nightly and cargo-fuzz; this test guarantees
//! that the checked-in seeds still parse without panicking on the stable
//! toolchain used by the normal test suite.

use std::path::Path;

use stc_manifest::{parse_lockfile, parse_manifest};
use stc_span::FileId;

#[test]
fn committed_manifest_seeds_parse_without_panic() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("fuzz")
        .join("seeds")
        .join("fuzz_manifest");
    let entries = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    let mut count = 0;
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if !path.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let file = FileId::new(0);
        let _ = parse_manifest(&text, file);
        let _ = parse_lockfile(&text, file);
        count += 1;
    }
    assert!(count >= 5, "expected committed manifest seeds");
}

#[test]
fn arbitrary_bytes_never_panic_in_the_manifest_parsers() {
    let inputs: [&[u8]; 5] = [
        b"",
        b"\xff\xfe\x00",
        b"schema = 1\n[package]\nname = \"x\"\n",
        b"\xef\xbb\xbfschema = 1\n",
        b"schema = 1\n[[package]]\nsource = { path = \"\xc3\xa9\" }\n",
    ];
    for input in inputs {
        if let Ok(text) = std::str::from_utf8(input) {
            let _ = parse_manifest(text, FileId::new(0));
            let _ = parse_lockfile(text, FileId::new(0));
        }
    }
}
