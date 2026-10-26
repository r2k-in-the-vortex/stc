//! Project manifest, source discovery, dependency graph, and `stc.lock`.
//!
//! This crate is the project-system front end from CLI and Manifest §2–§3. It
//! parses `stc.toml` into an explicit model, resolves local path dependencies
//! into a deterministic topological graph, discovers package sources, and
//! reads, checks, and writes the committed lockfile.
//!
//! The driver consumes [`ProjectGraph`]: packages are ordered dependencies
//! first, every package carries its manifest path, root directory, production
//! and test sources, dependency aliases, and validated build settings.

#![forbid(unsafe_code)]

mod codes;
mod discover;
mod glob;
mod graph;
mod lock;
mod model;
mod parse;

pub use graph::{
    LoadOptions, Package, PackageDependency, PackageId, ProjectError, ProjectGraph, load_project,
};
pub use lock::{LockPackage, LockSource, Lockfile, parse_lockfile, render_lockfile};
pub use model::{
    Baseline, BuildSettings, ChannelRef, ColdPolicy, ConfigValue, DebugProtocol, DebugSettings,
    DependencyDecl, DurationNs, EmitKind, GlobSpec, IoBackend, IoBinding, IoChannel, IoConfig,
    IoDevice, IoDirection, LOCKFILE_SCHEMA, LibType, LintLevel, LintSettings, MANIFEST_SCHEMA,
    Manifest, MismatchPolicy, OnlineSettings, OptLevel, PROFILE_ID, PackageInfo, ProjectSettings,
    RetainSettings, RunSettings, RuntimeSettings, SourcePath, SyntaxMode, TestCase, TestSettings,
};
pub use parse::{ParsedManifest, parse_manifest};
