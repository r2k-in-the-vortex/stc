//! `stc new`: scaffold a project (CLI-and-Manifest §5).

use std::path::Path;

use crate::report::Log;

const MAIN_SOURCE: &str = "\
PROGRAM Main
VAR
  counter : DINT;
END_VAR
  counter := counter + 1;
END_PROGRAM
";

const SMOKE_TEST: &str = "\
{stc:test}
FUNCTION SmokeTest : BOOL
  SmokeTest := TRUE;
END_FUNCTION
";

/// Creates the scaffold in `path`.
pub(crate) fn run(path: &str, log: Log) -> u8 {
    let target = Path::new(path);
    let Some(name) = target.file_name().and_then(|name| name.to_str()) else {
        eprintln!("stc: `{path}` is not a valid project path");
        return 2;
    };
    if !valid_name(name) {
        eprintln!(
            "stc: `{name}` is not a valid package name \
             (expected `[A-Za-z_][A-Za-z0-9_-]*`)"
        );
        return 2;
    }
    if target.exists() {
        if !target.is_dir() {
            eprintln!("stc: `{path}` already exists and is not a directory");
            return 2;
        }
        match std::fs::read_dir(target) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    eprintln!("stc: `{path}` already exists and is not empty");
                    return 2;
                }
            }
            Err(error) => {
                eprintln!("stc: cannot read `{path}`: {error}");
                return 2;
            }
        }
    } else if let Err(error) = std::fs::create_dir_all(target) {
        eprintln!("stc: cannot create `{path}`: {error}");
        return 2;
    }

    let manifest = format!(
        "schema = 1\n\
         \n\
         [package]\n\
         name = \"{name}\"\n\
         version = \"0.1.0\"\n\
         language = \"{}\"\n\
         \n\
         [project]\n\
         sources = [\"src/**/*.st\"]\n\
         \n\
         [test]\n\
         sources = [\"tests/**/*.st\"]\n",
        stc_manifest::PROFILE_ID
    );
    let files = [
        ("stc.toml", manifest.as_str()),
        ("src/main.st", MAIN_SOURCE),
        ("tests/smoke.st", SMOKE_TEST),
        (".gitignore", "/target/\n"),
    ];
    for (relative, _) in files {
        if target.join(relative).exists() {
            eprintln!("stc: `{}` already exists", target.join(relative).display());
            return 2;
        }
    }
    for (relative, contents) in files {
        let file = target.join(relative);
        if let Some(parent) = file.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            eprintln!("stc: cannot create `{}`: {error}", parent.display());
            return 2;
        }
        if let Err(error) = std::fs::write(&file, contents) {
            eprintln!("stc: cannot write `{}`: {error}", file.display());
            return 2;
        }
    }
    log.status(format!("created {}", target.display()));
    0
}

fn valid_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    characters
        .all(|character| character.is_ascii_alphanumeric() || character == '_' || character == '-')
}
