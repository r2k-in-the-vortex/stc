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
