#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use stc_diag::Diagnostic;
use stc_manifest::{LoadOptions, ProjectError, ProjectGraph, load_project, load_project_readonly};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A temporary project directory removed on drop.
pub struct TempProject {
    root: PathBuf,
}

impl TempProject {
    pub fn new(name: &str) -> Self {
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "stc-manifest-{}-{name}-{unique}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp project");
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        relative
            .split('/')
            .fold(self.root.clone(), |path, part| path.join(part))
    }

    pub fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create source directory");
        }
        std::fs::write(&path, contents).expect("write test file");
        path
    }

    pub fn manifest(&self, contents: &str) -> PathBuf {
        self.write("stc.toml", contents)
    }

    pub fn load(&self) -> Result<ProjectGraph, ProjectError> {
        load_project(self.root(), &LoadOptions::default())
    }

    pub fn load_locked(&self) -> Result<ProjectGraph, ProjectError> {
        load_project(self.root(), &LoadOptions { locked: true })
    }

    pub fn load_readonly(&self) -> Result<ProjectGraph, ProjectError> {
        load_project_readonly(self.root(), &LoadOptions::default())
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A minimal manifest for a package named `name`.
pub fn manifest_with(name: &str, sources: &str) -> String {
    format!(
        r#"schema = 1

[package]
name = "{name}"
version = "0.1.0"
language = "stc-2013-oop-utf8-v1"

[project]
sources = ["{sources}"]
"#
    )
}

pub fn codes(error: &ProjectError) -> Vec<&'static str> {
    error.codes().collect()
}

pub fn code_list(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

pub const MAIN_ST: &str =
    "PROGRAM Main\nVAR\n  counter : DINT;\nEND_VAR\n  counter := counter + 1;\nEND_PROGRAM\n";
