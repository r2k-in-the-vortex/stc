# Fuzzing

`fuzz_lexer` feeds arbitrary bytes to `stc-lexer` and checks the coverage, losslessness,
Eof, and no-panic contracts (STC-LEX-002, STC-LEX-004).

`fuzz_parser` lexes arbitrary bytes, checks the token-stream invariants, parses with both
semicolon modes (selected from the first input byte), and checks that the CST invariants hold and
`cst.text()` reproduces the decoded source exactly.

```sh
cargo install cargo-fuzz --locked
cargo +nightly fuzz run fuzz_lexer -- -max_total_time=60 -timeout=25 -rss_limit_mb=2048
cargo +nightly fuzz run fuzz_parser -- -max_total_time=60 -timeout=25 -rss_limit_mb=2048
```

CI runs each target for 60 seconds on Linux.

On Windows, the fuzzer links the dynamic AddressSanitizer runtime. If startup fails with
`STATUS_DLL_NOT_FOUND` or `STATUS_ENTRYPOINT_NOT_FOUND`, use the
`clang_rt.asan_dynamic-x86_64.dll` from the MSVC Build Tools directory that matches the Rust
nightly's LLVM version, either on `PATH` or next to the built `fuzz_lexer.exe` in
`fuzz/target/x86_64-pc-windows-msvc/release/`. The LLVM installation under
`C:\Program Files\LLVM` may lack the C++ interceptor exports that rustc's sanitizer build
expects.

`fuzz_codegen` parses and analyzes arbitrary input, then runs `stc_codegen::generate` twice when
the front end is clean. It checks HIR invariants, determinism, bounded output (128 MiB), sorted
span IDs, and a bounded error count. It never invokes clang.

```sh
cargo +nightly fuzz run fuzz_codegen -- -max_total_time=60 -timeout=25 -rss_limit_mb=2048
```

`fuzz_got_manifest` decodes arbitrary binary or canonical JSON GOT manifests, validates them, and
checks that the digest chain and both projections round-trip. Non-binary inputs are also wrapped
in a valid binary header with a recomputed length and digest so arbitrary bytes reach the section
and record decoders. Binary inputs that decode must re-encode byte-for-byte. The target never
loads a module or executes generated code.

`fuzz_module_descriptor` decodes arbitrary descriptor blobs and parses arbitrary UTF-8 module
metadata sidecars. Other inputs are wrapped in a valid descriptor header with a recomputed
declared size so arbitrary bytes reach the record decoders. Accepted descriptors must re-encode
and round-trip with a stable digest. The target never maps, loads, or executes native module code.

```sh
cargo +nightly fuzz run fuzz_got_manifest -- -max_total_time=60 -timeout=25 -rss_limit_mb=2048
cargo +nightly fuzz run fuzz_module_descriptor -- -max_total_time=60 -timeout=25 -rss_limit_mb=2048
```

The seed corpus under `corpus/` is ignored by git, like the other targets. CI rebuilds seeds for
both artifact targets from the committed codec snapshots and adds `-max_len=1048576` so an input
cannot exceed one MiB.
