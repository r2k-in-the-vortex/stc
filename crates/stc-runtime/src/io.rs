//! Process-image backends (`null`, `memory`, `file`) and the per-scan I/O
//! boundary (IO-and-Devices §6).
//!
//! At the start of every scan the host writes type defaults into the input
//! area and overlays the backend's input values; outputs are published once
//! after the module cycle. The `file` backend reads its input image once at
//! cold start and writes the last published output image atomically at
//! shutdown. The `null` backend discards outputs and gives unbound inputs
//! their defaults; the `memory` backend leaves host-supplied image bytes
//! alone.

use std::path::{Path, PathBuf};

use crate::module_info::{ImageLayout, ModuleInfo};
use crate::run::IoBackend;
use crate::state;

/// A non-fatal I/O notice produced during a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IoNotice {
    /// Stable diagnostic code.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
}

/// The host's process-image backend state.
#[derive(Clone, Debug)]
pub struct IoRuntime {
    backend: IoBackend,
    file: Option<PathBuf>,
    explicit_backend: bool,
    explicit_file: bool,
    layout: ImageLayout,
    input_defaults: Vec<u8>,
    output_defaults: Vec<u8>,
    /// Input image latched once at cold start for the `file` backend.
    latched_input: Vec<u8>,
    /// Last output image published by the `file` backend.
    output_image: Vec<u8>,
    notices: Vec<IoNotice>,
}

impl Default for IoRuntime {
    fn default() -> Self {
        Self {
            backend: IoBackend::Null,
            file: None,
            explicit_backend: false,
            explicit_file: false,
            layout: ImageLayout::default(),
            input_defaults: Vec::new(),
            output_defaults: Vec::new(),
            latched_input: Vec::new(),
            output_image: Vec::new(),
            notices: Vec::new(),
        }
    }
}

impl IoRuntime {
    /// Selects a backend or file explicitly (CLI override).
    ///
    /// An explicit backend or file survives metadata changes; it is not
    /// replaced by a later generation's project settings.
    pub fn configure(&mut self, backend: Option<IoBackend>, file: Option<PathBuf>) {
        if let Some(backend) = backend {
            self.backend = backend;
            self.explicit_backend = true;
        }
        if let Some(file) = file {
            self.file = Some(file);
            self.explicit_file = true;
        }
    }

    /// Applies one generation's metadata.
    ///
    /// `module_dir` is the base for a relative `[io].file` (D-163). Defaults
    /// belong to the module and are always adopted; the backend and file are
    /// adopted only when the caller did not override them.
    pub fn apply_metadata(&mut self, info: &ModuleInfo, module_dir: &Path) {
        self.layout = info.image_layout();
        self.input_defaults = sized(&info.input_defaults, self.layout.inputs_bytes);
        self.output_defaults = sized(&info.output_defaults, self.layout.outputs_bytes);
        if !self.explicit_backend {
            self.backend = info.backend;
        }
        if !self.explicit_file {
            self.file = match self.backend {
                IoBackend::File => info.io_file.as_deref().map(|file| module_dir.join(file)),
                _ => None,
            };
        }
    }

    /// Returns the effective backend.
    #[must_use]
    pub fn backend(&self) -> IoBackend {
        self.backend
    }

    /// Returns the effective file path.
    #[must_use]
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    /// Takes the notices recorded since the last call.
    #[must_use]
    pub fn take_notices(&mut self) -> Vec<IoNotice> {
        std::mem::take(&mut self.notices)
    }

    fn notice(&mut self, code: &'static str, message: impl Into<String>) {
        self.notices.push(IoNotice {
            code,
            message: message.into(),
        });
    }

    /// Latches the `file` backend input image and reports a missing file.
    ///
    /// Called once after a cold install's prepare has applied declaration
    /// defaults, so a missing input file starts from those defaults.
    pub fn cold_start(&mut self) {
        if self.backend != IoBackend::File {
            self.latched_input = self.input_defaults.clone();
            return;
        }
        let bytes = self.layout.inputs_bytes;
        let defaults = self.default_input_image();
        let Some(path) = self.file.clone() else {
            self.notice(
                "STC-W-IO-004",
                "the file I/O backend has no configured file; inputs use defaults",
            );
            self.latched_input = defaults;
            return;
        };
        match std::fs::read(&path) {
            Ok(contents) => {
                // A short file supplies the leading bytes; the remaining
                // locations keep their defaults.
                let mut latched = defaults;
                let count = latched
                    .len()
                    .min(contents.len())
                    .min(usize::try_from(bytes).unwrap_or(0));
                latched[..count].copy_from_slice(&contents[..count]);
                self.latched_input = latched;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.notice(
                    "STC-W-IO-004",
                    format!(
                        "I/O input file `{}` was not found; starting with defaults",
                        path.display()
                    ),
                );
                self.latched_input = defaults;
            }
            Err(error) => {
                self.notice(
                    "STC-W-IO-004",
                    format!(
                        "cannot read I/O input file `{}`: {error}; starting with defaults",
                        path.display()
                    ),
                );
                self.latched_input = defaults;
            }
        }
    }

    /// Applies input defaults and overlays the backend's inputs.
    ///
    /// Called after the scan time is latched and before task guards are set,
    /// so every due task sees the same input image.
    pub fn latch_inputs(&mut self) {
        let inputs = usize::try_from(self.layout.inputs_bytes).unwrap_or(0);
        if inputs == 0 {
            return;
        }
        let defaults = sized(&self.input_defaults, self.layout.inputs_bytes);
        match self.backend {
            IoBackend::Null => {
                crate::image_with(|image| {
                    let area = &mut image[..inputs];
                    area.copy_from_slice(&defaults);
                });
            }
            IoBackend::Memory => {
                // Host embedding owns the input bytes: capture them, apply
                // defaults for unwritten locations, then restore the writes.
                crate::image_with(|image| {
                    let area = &mut image[..inputs];
                    let supplied = area.to_vec();
                    area.copy_from_slice(&defaults);
                    area.copy_from_slice(&supplied);
                });
            }
            IoBackend::File => {
                let latched = sized(&self.latched_input, self.layout.inputs_bytes);
                crate::image_with(|image| {
                    let area = &mut image[..inputs];
                    area.copy_from_slice(&latched);
                });
            }
        }
    }

    /// Publishes the output image once after a successful module cycle.
    pub fn publish_outputs(&mut self) {
        let inputs = usize::try_from(self.layout.inputs_bytes).unwrap_or(0);
        let outputs = usize::try_from(self.layout.outputs_bytes).unwrap_or(0);
        if outputs == 0 {
            return;
        }
        match self.backend {
            IoBackend::Null => {
                crate::image_with(|image| {
                    let start = inputs.min(image.len());
                    let end = start.saturating_add(outputs).min(image.len());
                    image[start..end].fill(0);
                });
            }
            IoBackend::Memory => {}
            IoBackend::File => {
                crate::image_with(|image| {
                    let start = inputs.min(image.len());
                    let end = start.saturating_add(outputs).min(image.len());
                    self.output_image.clear();
                    self.output_image.extend_from_slice(&image[start..end]);
                });
            }
        }
    }

    /// Writes the `file` backend's last output image atomically.
    ///
    /// Returns an error message when the file cannot be written; the previous
    /// file is left intact.
    pub fn flush_output(&mut self) -> Option<String> {
        if self.backend != IoBackend::File {
            return None;
        }
        let Some(path) = self.file.as_deref() else {
            return Some("the file I/O backend has no configured output file".into());
        };
        let bytes = if self.output_image.is_empty() {
            // No cycle ran: publish the image's current output area.
            let inputs = usize::try_from(self.layout.inputs_bytes).unwrap_or(0);
            let outputs = usize::try_from(self.layout.outputs_bytes).unwrap_or(0);
            let mut buffer = vec![0; outputs];
            let count = crate::image_read(inputs, &mut buffer);
            buffer[..count].to_vec()
        } else {
            self.output_image.clone()
        };
        state::write_atomically(path, &bytes)
            .err()
            .map(|error| format!("cannot write I/O output file `{}`: {error}", path.display()))
    }

    /// Returns the input image defaults sized to the input area.
    fn default_input_image(&self) -> Vec<u8> {
        sized(&self.input_defaults, self.layout.inputs_bytes)
    }
}

fn sized(bytes: &[u8], length: u64) -> Vec<u8> {
    let mut value = vec![0; usize::try_from(length).unwrap_or(0)];
    let count = value.len().min(bytes.len());
    value[..count].copy_from_slice(&bytes[..count]);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_resized_to_the_area() {
        assert_eq!(sized(&[1, 2], 4), vec![1, 2, 0, 0]);
        assert_eq!(sized(&[1, 2, 3], 2), vec![1, 2]);
        assert_eq!(sized(&[], 0), Vec::<u8>::new());
    }
}
