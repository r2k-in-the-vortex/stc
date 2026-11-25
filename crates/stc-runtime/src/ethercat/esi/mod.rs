//! ESI ingestion: bounded reader, typed capability model and a deterministic
//! local library.
//!
//! ESI files are a configuration-time input only: once a project persists its
//! desired configuration, builds and runtime do not read ESI files (D-263).

pub mod catalog;
pub mod model;
pub mod reader;
