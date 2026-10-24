//! Host toolchain discovery for linking on Windows MSVC (D-078).

#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use std::process::Command;

/// Returns the linker arguments needed to link a Rust staticlib and generated
/// object with clang on this host.
#[must_use]
pub fn system_link_args() -> Vec<String> {
    #[cfg(windows)]
    {
        windows_link_args()
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        {
            Vec::new()
        }
        #[cfg(not(target_os = "macos"))]
        {
            // Rust std staticlibs on GNU/Linux need the platform thread,
            // math, and dynamic-loading libraries.
            vec!["-ldl".into(), "-lpthread".into(), "-lm".into()]
        }
    }
}

#[cfg(windows)]
fn windows_link_args() -> Vec<String> {
    let mut args = Vec::new();
    if let Some(directory) = msvc_lib_directory() {
        args.push("-Xlinker".into());
        args.push(format!("/LIBPATH:{}", directory.display()));
    }
    for directory in sdk_lib_directories() {
        args.push("-Xlinker".into());
        args.push(format!("/LIBPATH:{}", directory.display()));
    }
    for library in [
        "ws2_32", "ntdll", "userenv", "advapi32", "bcrypt", "ole32", "shell32", "user32", "dbghelp",
    ] {
        args.push(format!("-l{library}"));
    }
    args
}

#[cfg(windows)]
fn program_files_roots() -> Vec<PathBuf> {
    [
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from),
        std::env::var_os("ProgramFiles").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    .collect()
}

#[cfg(windows)]
fn msvc_lib_directory() -> Option<PathBuf> {
    for root in program_files_roots() {
        let vswhere = root
            .join("Microsoft Visual Studio")
            .join("Installer")
            .join("vswhere.exe");
        if vswhere.is_file()
            && let Ok(output) = Command::new(&vswhere)
                .args([
                    "-latest",
                    "-products",
                    "*",
                    "-requires",
                    "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                    "-property",
                    "installationPath",
                ])
                .output()
            && output.status.success()
        {
            let install = String::from_utf8_lossy(&output.stdout);
            let install = PathBuf::from(install.trim());
            if let Some(directory) = highest_version(&install.join("VC/Tools/MSVC")) {
                let lib = directory.join("lib/x64");
                if lib.is_dir() {
                    return Some(lib);
                }
            }
        }
    }
    for root in program_files_roots() {
        let Ok(years) = std::fs::read_dir(root.join("Microsoft Visual Studio")) else {
            continue;
        };
        for year in years.flatten() {
            let Ok(editions) = std::fs::read_dir(year.path()) else {
                continue;
            };
            for edition in editions.flatten() {
                if let Some(directory) = highest_version(&edition.path().join("VC/Tools/MSVC")) {
                    let lib = directory.join("lib/x64");
                    if lib.is_dir() {
                        return Some(lib);
                    }
                }
            }
        }
    }
    None
}

#[cfg(windows)]
fn highest_version(base: &std::path::Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    let mut versions: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    versions.pop()
}

#[cfg(windows)]
fn sdk_lib_directories() -> Vec<PathBuf> {
    let Some(root) = std::env::var_os("ProgramFiles(x86)")
        .map(PathBuf::from)
        .map(|root| root.join("Windows Kits/10/Lib"))
    else {
        return Vec::new();
    };
    let Some(version) = highest_version(&root) else {
        return Vec::new();
    };
    let mut directories = Vec::new();
    for part in ["ucrt/x64", "um/x64"] {
        let directory = version.join(part);
        if directory.is_dir() {
            directories.push(directory);
        }
    }
    directories
}
