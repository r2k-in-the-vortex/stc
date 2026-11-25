#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The reader must return a diagnostic for arbitrary input and never panic,
    // hang or allocate without a bound (WL-7 plan §7).
    stc_runtime::ethercat::esi::reader::parse_bytes_for_fuzz(data);
});
