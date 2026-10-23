# Preface

Goal is to generate IEC 61131-3 latest edition Structured Text to llvm compiler.

The compiler itself will be written in Rust.

Before writing any compiler code, extensive specification will be generated in Specification folder.
Readable markdown files are acceptable.

At minimum: Tokenization scheme, Archidecture, AST and Syntax will be specified.

For any math functions, Lean specifications will be developed before implementation in code.

Syntax inspiration will be Simatix AX, because it has particularly well crafted and public documentation.
https://docs.industrial-operations-x.siemens.cloud/r/en-us/ax/st-docs/11.7.50/structured-text-language-reference

The compiler will be a cli tool, with verbose output and strict insistence on code correctness, similar to cargo

It will have built in test suite functionality.

The pipeline is standard:
Lexer -> Parser -> Analyzer -> Codegen

ST project structure and options will be defined as toml file

# Project folder structure

[AI fill out]

# Dev environment

PS C:\repos\stc> clang -v
clang version 23.1.4 (https://github.com/llvm/llvm-project 7a941dfb2877a0b8cf30323fefa85ad019e0b0f5)
Target: x86_64-pc-windows-msvc
Thread model: posix
InstalledDir: C:\Program Files\LLVM\bin
Build config: +alloc:rpmalloc

PS C:\repos\stc> lake --version
Lake version 5.0.0-src+eb6cafb (Lean version 4.35.0)

PS C:\repos\stc> cargo -V
cargo 1.99.1 (5f94df478 2026-08-27)