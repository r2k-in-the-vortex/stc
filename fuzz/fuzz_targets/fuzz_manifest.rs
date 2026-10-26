#![no_main]

//! Fuzz the project-manifest and lockfile parsers.
//!
//! Every UTF-8 input is parsed as both `stc.toml` and `stc.lock`. Parsing is
//! pure: the target never touches the filesystem, resolves dependencies,
//! writes a lockfile, or executes generated code. Success and failure are both
//! fine; the contracts under test are no-panic, bounded recursion, and
//! deterministic diagnostics.

use libfuzzer_sys::fuzz_target;
use stc_manifest::{parse_lockfile, parse_manifest};
use stc_span::FileId;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let file = FileId::new(0);
    if let Ok(parsed) = parse_manifest(text, file) {
        let manifest = &parsed.manifest;
        // Touch the model so the compiler cannot elide validation work.
        let _ = manifest.package.name.len()
            + manifest.project.sources.len()
            + manifest.dependencies.len()
            + manifest.io.devices.len()
            + manifest.io.bindings.len()
            + manifest.test.cases.len();
    }
    let _ = parse_lockfile(text, file);
});
