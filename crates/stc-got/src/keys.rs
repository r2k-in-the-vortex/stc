//! Normalized stable keys and identity kinds (GOT Artifacts §3).

use crate::error::{Error, Result};
use crate::layout::Signature;

/// The persistent identity kind tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IdentityKind {
    /// A persistent data root.
    DataRoot = 1,
    /// A callable function, program body, method, FB body, or standard function.
    Function = 2,
    /// A named user-defined type.
    Type = 3,
    /// A runtime descriptor or vtable.
    Descriptor = 4,
    /// A configured program instance.
    Instance = 5,
    /// A task.
    Task = 6,
    /// A C export.
    Export = 7,
    /// A type field identity.
    Field = 8,
    /// A direct process-image variable.
    Direct = 9,
}

impl IdentityKind {
    /// Returns the binary identity tag.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Parses an identity tag.
    #[must_use]
    pub const fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::DataRoot),
            2 => Some(Self::Function),
            3 => Some(Self::Type),
            4 => Some(Self::Descriptor),
            5 => Some(Self::Instance),
            6 => Some(Self::Task),
            7 => Some(Self::Export),
            8 => Some(Self::Field),
            9 => Some(Self::Direct),
            _ => None,
        }
    }

    /// Returns a human-readable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::DataRoot => "data root",
            Self::Function => "function",
            Self::Type => "type",
            Self::Descriptor => "descriptor",
            Self::Instance => "instance",
            Self::Task => "task",
            Self::Export => "export",
            Self::Field => "field",
            Self::Direct => "direct variable",
        }
    }

    /// Returns the key form description used in diagnostics.
    #[must_use]
    pub const fn key_form(self) -> &'static str {
        match self {
            Self::DataRoot => "PKG::PATH[#INSTANCE]",
            Self::Function => "PKG::PATH@SIGNATURE",
            Self::Type => "PKG::PATH",
            Self::Descriptor => "PKG::TYPE#DESC:KIND",
            Self::Instance => "PKG::CONFIG/RESOURCE/INSTANCE",
            Self::Task => "PKG::CONFIG/RESOURCE#TASK",
            Self::Export => "PKG::PATH@SIGNATURE#C:C_NAME",
            Self::Field => "PKG::TYPE_PATH.FIELD_PATH",
            Self::Direct => "PKG::PATH[#INSTANCE]",
        }
    }
}

/// Normalizes and validates a package name.
///
/// Package names accept `-` and are stored uppercase (ASCII case folding).
///
/// # Errors
///
/// Rejects an empty name or characters outside the grammar.
pub fn normalize_package(name: &str) -> Result<String> {
    if name.is_empty() {
        return Err(Error::manifest("package name is empty"));
    }
    let mut out = String::with_capacity(name.len());
    for (index, character) in name.chars().enumerate() {
        let valid = if index == 0 {
            character.is_ascii_alphabetic() || character == '_'
        } else {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        };
        if !valid {
            return Err(Error::manifest(format!(
                "package name `{name}` contains an invalid character"
            )));
        }
        out.push(character.to_ascii_uppercase());
    }
    Ok(out)
}

/// Normalizes and validates one identifier.
///
/// # Errors
///
/// Rejects an empty name or characters outside `[A-Za-z_][A-Za-z0-9_]*`.
pub fn normalize_ident(name: &str) -> Result<String> {
    if name.is_empty() {
        return Err(Error::manifest("identifier is empty"));
    }
    let mut out = String::with_capacity(name.len());
    for (index, character) in name.chars().enumerate() {
        let valid = if index == 0 {
            character.is_ascii_alphabetic() || character == '_'
        } else {
            character.is_ascii_alphanumeric() || character == '_'
        };
        if !valid {
            return Err(Error::manifest(format!(
                "identifier `{name}` contains an invalid character"
            )));
        }
        out.push(character.to_ascii_uppercase());
    }
    Ok(out)
}

/// Normalizes a dot-separated name path.
///
/// # Errors
///
/// Rejects empty segments or invalid identifiers.
pub fn normalize_name_path(path: &str) -> Result<String> {
    normalize_joined(path, '.', "name path")
}

/// Normalizes a slash-separated instance path.
///
/// # Errors
///
/// Rejects empty segments or invalid identifiers.
pub fn normalize_instance_path(path: &str) -> Result<String> {
    normalize_joined(path, '/', "instance path")
}

fn normalize_joined(path: &str, separator: char, what: &str) -> Result<String> {
    if path.is_empty() {
        return Err(Error::manifest(format!("{what} is empty")));
    }
    let mut out = String::with_capacity(path.len());
    for (index, segment) in path.split(separator).enumerate() {
        if index > 0 {
            out.push(separator);
        }
        out.push_str(&normalize_ident(segment)?);
    }
    Ok(out)
}

/// Returns the package portion of a key.
#[must_use]
pub fn key_package(key: &str) -> Option<&str> {
    key.split_once("::").map(|(package, _)| package)
}

/// Builds a global data-root key.
///
/// # Errors
///
/// Rejects malformed package or path components.
pub fn global_root_key(package: &str, path: &str) -> Result<String> {
    Ok(format!(
        "{}::{}",
        normalize_package(package)?,
        normalize_name_path(path)?
    ))
}

/// Builds a program-instance field-root key.
///
/// # Errors
///
/// Rejects malformed package, path, or instance components.
pub fn field_root_key(package: &str, path: &str, instance: &str) -> Result<String> {
    Ok(format!(
        "{}::{}#{}",
        normalize_package(package)?,
        normalize_name_path(path)?,
        normalize_instance_path(instance)?
    ))
}

/// Builds a task-guard root key.
///
/// # Errors
///
/// Rejects malformed package, configuration, resource, or task names.
pub fn task_guard_root_key(
    package: &str,
    configuration: &str,
    resource: &str,
    task: &str,
) -> Result<String> {
    Ok(format!(
        "{}::{}/{}#{}.GUARD",
        normalize_package(package)?,
        normalize_ident(configuration)?,
        normalize_ident(resource)?,
        normalize_ident(task)?
    ))
}

/// Reserved hidden-root detail markers (GOT Artifacts §8.5.6).
///
/// A hidden object-record or class-handle root appends one of these markers as
/// the final segment of the key's `#` detail suffix. The markers never occur in
/// a source identifier position.
pub const HIDDEN_ROOT_MARKERS: [&str; 2] = ["HANDLE", "OBJREC"];

/// Returns the hidden-root marker named by a data-root key, when present.
///
/// A marker is recognized only when it is unambiguous: either the whole detail
/// suffix (`PATH#HANDLE`) or the final segment of a complete instance path
/// (`PATH#CONFIG/RESOURCE/INSTANCE/HANDLE`). A three-segment ordinary instance
/// path may legitimately end in an instance named `HANDLE`; that key has no
/// hidden marker.
#[must_use]
pub fn hidden_root_marker(key: &str) -> Option<&'static str> {
    let (_, suffix) = key.split_once('#')?;
    if let Some(marker) = HIDDEN_ROOT_MARKERS
        .iter()
        .copied()
        .find(|marker| suffix == *marker)
    {
        return Some(marker);
    }
    let (instance, marker) = suffix.rsplit_once('/')?;
    if instance.split('/').count() < 3 {
        return None;
    }
    HIDDEN_ROOT_MARKERS
        .iter()
        .copied()
        .find(|candidate| marker == *candidate)
}

/// Builds a hidden object-record or class-handle root key.
///
/// `instance` is the declaration's instance path, present for a program-instance
/// field and absent for a global. The marker follows the detail suffix.
///
/// # Errors
///
/// Rejects an unknown marker and malformed package, path, or instance
/// components.
pub fn hidden_root_key(
    package: &str,
    path: &str,
    instance: Option<&str>,
    marker: &str,
) -> Result<String> {
    let marker = HIDDEN_ROOT_MARKERS
        .iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(marker))
        .ok_or_else(|| {
            Error::manifest(format!(
                "unknown hidden-root marker `{marker}` (expected HANDLE or OBJREC)"
            ))
        })?;
    let package = normalize_package(package)?;
    let path = normalize_name_path(path)?;
    match instance {
        Some(instance) => Ok(format!(
            "{package}::{path}#{}/{marker}",
            normalize_instance_path(instance)?
        )),
        None => Ok(format!("{package}::{path}#{marker}")),
    }
}

/// Builds a direct-variable root key.
///
/// `instance` is present for a program-instance field and absent for a global.
///
/// # Errors
///
/// Rejects malformed package, path, or instance components.
pub fn direct_root_key(package: &str, path: &str, instance: Option<&str>) -> Result<String> {
    let package = normalize_package(package)?;
    let path = normalize_name_path(path)?;
    match instance {
        Some(instance) => Ok(format!(
            "{package}::{path}#{}",
            normalize_instance_path(instance)?
        )),
        None => Ok(format!("{package}::{path}")),
    }
}

/// Builds a function key.
///
/// # Errors
///
/// Rejects malformed package or function path components.
pub fn function_key(package: &str, path: &str, signature: &Signature) -> Result<String> {
    Ok(format!(
        "{}::{}@{}",
        normalize_package(package)?,
        normalize_name_path(path)?,
        signature
    ))
}

/// Builds a program-body key.
///
/// The program body has the fixed signature `()->VOID`.
///
/// # Errors
///
/// Rejects malformed package or type path components.
pub fn program_body_key(package: &str, type_path: &str) -> Result<String> {
    Ok(format!(
        "{}::{}#BODY@()->VOID",
        normalize_package(package)?,
        normalize_name_path(type_path)?
    ))
}

/// Builds a type key.
///
/// # Errors
///
/// Rejects malformed package or type path components.
pub fn type_identity_key(package: &str, path: &str) -> Result<String> {
    Ok(format!(
        "{}::{}",
        normalize_package(package)?,
        normalize_name_path(path)?
    ))
}

/// Builds a type-field identity key.
///
/// # Errors
///
/// Rejects malformed package, type path, or field path components.
pub fn field_identity_key(package: &str, type_path: &str, field_path: &str) -> Result<String> {
    Ok(format!(
        "{}::{}.{}",
        normalize_package(package)?,
        normalize_name_path(type_path)?,
        normalize_name_path(field_path)?
    ))
}

/// Builds a descriptor key.
///
/// # Errors
///
/// Rejects malformed package or type path components and unknown descriptor
/// kinds.
pub fn descriptor_identity_key(package: &str, type_path: &str, kind: &str) -> Result<String> {
    let kind = normalize_descriptor_kind(kind)?;
    Ok(format!(
        "{}::{}#DESC:{kind}",
        normalize_package(package)?,
        normalize_name_path(type_path)?
    ))
}

/// Builds a configured-instance key.
///
/// # Errors
///
/// Rejects malformed package or instance components.
pub fn instance_identity_key(
    package: &str,
    configuration: &str,
    resource: &str,
    instance: &str,
) -> Result<String> {
    Ok(format!(
        "{}::{}/{}/{}",
        normalize_package(package)?,
        normalize_ident(configuration)?,
        normalize_ident(resource)?,
        normalize_ident(instance)?
    ))
}

/// Builds a task key.
///
/// # Errors
///
/// Rejects malformed package, configuration, resource, or task names.
pub fn task_identity_key(
    package: &str,
    configuration: &str,
    resource: &str,
    task: &str,
) -> Result<String> {
    Ok(format!(
        "{}::{}/{}#{}",
        normalize_package(package)?,
        normalize_ident(configuration)?,
        normalize_ident(resource)?,
        normalize_ident(task)?
    ))
}

/// Builds an export key.
///
/// The C name is case-sensitive and is kept verbatim after the `#C:` marker.
///
/// # Errors
///
/// Rejects malformed package, function path, or C name components.
pub fn export_identity_key(
    package: &str,
    path: &str,
    signature: &Signature,
    c_name: &str,
) -> Result<String> {
    validate_c_name(c_name)?;
    Ok(format!(
        "{}::{}@{}#C:{}",
        normalize_package(package)?,
        normalize_name_path(path)?,
        signature,
        c_name
    ))
}

fn normalize_descriptor_kind(kind: &str) -> Result<&'static str> {
    match kind.to_ascii_uppercase().as_str() {
        "VTABLE" => Ok("VTABLE"),
        "TYPEINFO" => Ok("TYPEINFO"),
        "OBJECT" => Ok("OBJECT"),
        _ => Err(Error::manifest(format!(
            "unknown descriptor kind `{kind}` (expected VTABLE, TYPEINFO, or OBJECT)"
        ))),
    }
}

/// Validates a stable key against the grammar for `kind`.
///
/// Keys must already be normalized (uppercase ASCII); this rejects a key that
/// is not in canonical form even when it would otherwise be understandable.
///
/// # Errors
///
/// Rejects every malformed form and mismatched identity kind.
pub fn validate_key(kind: IdentityKind, key: &str) -> Result<()> {
    let Some((package, rest)) = key.split_once("::") else {
        return Err(invalid_key(kind, key, "missing `::` package separator"));
    };
    validate_normalized_package(package).map_err(|error| invalid_key(kind, key, &error.message))?;
    if rest.is_empty() {
        return Err(invalid_key(kind, key, "empty path"));
    }
    if key.matches("::").count() != 1 {
        return Err(invalid_key(kind, key, "more than one `::` separator"));
    }
    match kind {
        IdentityKind::DataRoot => {
            validate_root_key(rest, true).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Direct => {
            validate_root_key(rest, false).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Function => {
            validate_function_key(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Type => {
            validate_name_path(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Descriptor => {
            validate_descriptor_key(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Instance => {
            validate_instance_key(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Task => {
            validate_task_key(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Export => {
            validate_export_key(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
        IdentityKind::Field => {
            validate_field_key(rest).map_err(|error| invalid_key(kind, key, &error.message))
        }
    }
}

fn invalid_key(kind: IdentityKind, key: &str, reason: &str) -> Error {
    Error::manifest(format!(
        "`{key}` is not a valid {} key ({}): expected {}",
        kind.name(),
        reason,
        kind.key_form()
    ))
}

fn validate_normalized_package(package: &str) -> Result<()> {
    let normalized = normalize_package(package)?;
    if normalized != package {
        return Err(Error::manifest(format!(
            "package name `{package}` is not normalized (`{normalized}`)"
        )));
    }
    Ok(())
}

fn validate_normalized_ident(text: &str) -> Result<()> {
    let normalized = normalize_ident(text)?;
    if normalized != text {
        return Err(Error::manifest(format!(
            "identifier `{text}` is not normalized (`{normalized}`)"
        )));
    }
    Ok(())
}

fn validate_name_path(path: &str) -> Result<()> {
    let normalized = normalize_name_path(path)?;
    if normalized != path {
        return Err(Error::manifest(format!(
            "name path `{path}` is not normalized"
        )));
    }
    Ok(())
}

fn validate_instance_path(path: &str) -> Result<usize> {
    let normalized = normalize_instance_path(path)?;
    if normalized != path {
        return Err(Error::manifest(format!(
            "instance path `{path}` is not normalized"
        )));
    }
    Ok(path.split('/').count())
}

fn validate_root_key(rest: &str, allow_task_guard: bool) -> Result<()> {
    match rest.split_once('#') {
        None => validate_name_path(rest),
        Some((path, suffix)) => {
            if suffix.contains('#') {
                return Err(Error::manifest("more than one `#` marker"));
            }
            if allow_task_guard && let Some(task) = suffix.strip_suffix(".GUARD") {
                let segments = validate_instance_path(path)?;
                if segments != 2 {
                    return Err(Error::manifest(
                        "a task-guard key is `CONFIG/RESOURCE#TASK.GUARD`",
                    ));
                }
                return validate_normalized_ident(task);
            }
            validate_name_path(path)?;
            // A hidden root appends `HANDLE` or `OBJREC` as the final detail
            // segment: `PATH#MARKER` for a global, `PATH#CONFIG/RESOURCE/
            // INSTANCE[/NESTED]/MARKER` for a configured instance.
            if hidden_root_marker(rest).is_some() {
                let Some((instance, _marker)) = suffix.rsplit_once('/') else {
                    return Ok(());
                };
                let segments = validate_instance_path(instance)?;
                if segments < 3 {
                    return Err(Error::manifest(
                        "a hidden root's instance path is `CONFIG/RESOURCE/INSTANCE`",
                    ));
                }
                return Ok(());
            }
            let segments = validate_instance_path(suffix)?;
            if segments < 3 {
                return Err(Error::manifest(
                    "an instance path is `CONFIG/RESOURCE/INSTANCE`",
                ));
            }
            Ok(())
        }
    }
}

fn validate_function_key(rest: &str) -> Result<()> {
    let Some((left, signature)) = rest.split_once('@') else {
        return Err(Error::manifest("missing `@signature`"));
    };
    if signature.contains('@') {
        return Err(Error::manifest("more than one `@` marker"));
    }
    let _signature = Signature::parse(signature)?;
    if let Some((type_path, marker)) = left.split_once('#') {
        if marker != "BODY" {
            return Err(Error::manifest("the only function-path marker is `#BODY`"));
        }
        validate_name_path(type_path)?;
        // Program bodies use the fixed `()->VOID` signature; function-block
        // bodies use the FB input/output parameter list (GOT Artifacts §4.2).
        Ok(())
    } else {
        validate_name_path(left)
    }
}

fn validate_descriptor_key(rest: &str) -> Result<()> {
    let Some((type_path, kind)) = rest.split_once("#DESC:") else {
        return Err(Error::manifest("missing `#DESC:KIND` marker"));
    };
    validate_name_path(type_path)?;
    normalize_descriptor_kind(kind)?;
    Ok(())
}

fn validate_instance_key(rest: &str) -> Result<()> {
    if rest.contains('#') || rest.contains('@') {
        return Err(Error::manifest(
            "an instance key contains only `/` segments",
        ));
    }
    let segments = validate_instance_path(rest)?;
    if segments < 3 {
        return Err(Error::manifest(
            "an instance key is `CONFIG/RESOURCE/INSTANCE`",
        ));
    }
    Ok(())
}

fn validate_task_key(rest: &str) -> Result<()> {
    let Some((resource, task)) = rest.split_once('#') else {
        return Err(Error::manifest("missing `#TASK` marker"));
    };
    if task.contains('#') || task.contains('@') {
        return Err(Error::manifest("more than one marker in a task key"));
    }
    let segments = validate_instance_path(resource)?;
    if segments != 2 {
        return Err(Error::manifest("a task key is `CONFIG/RESOURCE#TASK`"));
    }
    validate_normalized_ident(task)
}

fn validate_export_key(rest: &str) -> Result<()> {
    let Some((function, c_name)) = rest.rsplit_once("#C:") else {
        return Err(Error::manifest("missing `#C:C_NAME` marker"));
    };
    validate_c_name(c_name)?;
    let Some((path, signature)) = function.split_once('@') else {
        return Err(Error::manifest("missing `@signature`"));
    };
    validate_name_path(path)?;
    Signature::parse(signature)?;
    Ok(())
}

fn validate_field_key(rest: &str) -> Result<()> {
    if rest.contains('#') || rest.contains('@') {
        return Err(Error::manifest("a field key contains only `.` segments"));
    }
    let normalized = normalize_name_path(rest)?;
    if normalized != rest {
        return Err(Error::manifest(format!(
            "field path `{rest}` is not normalized"
        )));
    }
    if rest.split('.').count() < 2 {
        return Err(Error::manifest("a field key is `TYPE_PATH.FIELD_PATH`"));
    }
    Ok(())
}

fn validate_c_name(c_name: &str) -> Result<()> {
    if c_name.is_empty() {
        return Err(Error::manifest("C export name is empty"));
    }
    for (index, character) in c_name.chars().enumerate() {
        let valid = if index == 0 {
            character.is_ascii_alphabetic() || character == '_'
        } else {
            character.is_ascii_alphanumeric() || character == '_'
        };
        if !valid {
            return Err(Error::manifest(format!(
                "C export name `{c_name}` is not a C identifier"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{ParamMode, Parameter, TypeKey};

    fn signature() -> Signature {
        Signature {
            parameters: vec![Parameter {
                mode: ParamMode::In,
                ty: TypeKey::Dint,
            }],
            result: TypeKey::Dint,
        }
    }

    #[test]
    fn normalizes_letters_and_paths() {
        assert_eq!(normalize_package("_root").unwrap(), "_ROOT");
        assert_eq!(normalize_package("motor-control").unwrap(), "MOTOR-CONTROL");
        assert_eq!(normalize_ident("main").unwrap(), "MAIN");
        assert_eq!(
            normalize_name_path("globals.speed").unwrap(),
            "GLOBALS.SPEED"
        );
        assert_eq!(
            normalize_instance_path("main/r1/inst").unwrap(),
            "MAIN/R1/INST"
        );
        assert!(normalize_package("").is_err());
        assert!(normalize_package("1bad").is_err());
        assert!(normalize_ident("a-b").is_err());
        assert!(normalize_name_path("A..B").is_err());
    }

    #[test]
    fn builders_produce_valid_keys() {
        let signature = signature();
        let keys = [
            (
                IdentityKind::DataRoot,
                global_root_key("_root", "globals.speed").unwrap(),
                "_ROOT::GLOBALS.SPEED",
            ),
            (
                IdentityKind::DataRoot,
                field_root_key("_root", "main.speed", "main/r1/inst").unwrap(),
                "_ROOT::MAIN.SPEED#MAIN/R1/INST",
            ),
            (
                IdentityKind::DataRoot,
                task_guard_root_key("_root", "main", "r1", "fast").unwrap(),
                "_ROOT::MAIN/R1#FAST.GUARD",
            ),
            (
                IdentityKind::DataRoot,
                hidden_root_key("_root", "main.motor", None, "OBJREC").unwrap(),
                "_ROOT::MAIN.MOTOR#OBJREC",
            ),
            (
                IdentityKind::DataRoot,
                hidden_root_key("_root", "main.motor", Some("main/r1/inst"), "HANDLE").unwrap(),
                "_ROOT::MAIN.MOTOR#MAIN/R1/INST/HANDLE",
            ),
            (
                IdentityKind::DataRoot,
                hidden_root_key("_root", "main.arr.e1", Some("main/r1/inst/fb1"), "objrec")
                    .unwrap(),
                "_ROOT::MAIN.ARR.E1#MAIN/R1/INST/FB1/OBJREC",
            ),
            (
                IdentityKind::Direct,
                direct_root_key("_root", "main.input", Some("main/r1/inst")).unwrap(),
                "_ROOT::MAIN.INPUT#MAIN/R1/INST",
            ),
            (
                IdentityKind::Function,
                function_key("_root", "math.add", &signature).unwrap(),
                "_ROOT::MATH.ADD@(IN:DINT)->DINT",
            ),
            (
                IdentityKind::Function,
                program_body_key("_root", "main").unwrap(),
                "_ROOT::MAIN#BODY@()->VOID",
            ),
            (
                IdentityKind::Type,
                type_identity_key("_root", "types.motor").unwrap(),
                "_ROOT::TYPES.MOTOR",
            ),
            (
                IdentityKind::Field,
                field_identity_key("_root", "types.motor", "speed").unwrap(),
                "_ROOT::TYPES.MOTOR.SPEED",
            ),
            (
                IdentityKind::Descriptor,
                descriptor_identity_key("_root", "types.motor", "vtable").unwrap(),
                "_ROOT::TYPES.MOTOR#DESC:VTABLE",
            ),
            (
                IdentityKind::Instance,
                instance_identity_key("_root", "main", "r1", "inst").unwrap(),
                "_ROOT::MAIN/R1/INST",
            ),
            (
                IdentityKind::Task,
                task_identity_key("_root", "main", "r1", "fast").unwrap(),
                "_ROOT::MAIN/R1#FAST",
            ),
            (
                IdentityKind::Export,
                export_identity_key("_root", "api.read", &signature, "stc_read").unwrap(),
                "_ROOT::API.READ@(IN:DINT)->DINT#C:stc_read",
            ),
        ];
        for (kind, key, expected) in keys {
            assert_eq!(key, expected);
            validate_key(kind, &key).unwrap_or_else(|error| panic!("{key}: {error}"));
        }
    }

    #[test]
    fn rejects_cross_kind_and_malformed_keys() {
        assert!(validate_key(IdentityKind::Function, "_ROOT::MAIN.SPEED").is_err());
        assert!(validate_key(IdentityKind::Type, "_ROOT::MAIN#BODY@()->VOID").is_err());
        // Function-block bodies carry the FB parameter signature; program
        // bodies use `()->VOID`. Both are valid `#BODY` keys.
        validate_key(IdentityKind::Function, "_ROOT::MAIN#BODY@(IN:DINT)->DINT")
            .expect("FB body key");
        assert!(validate_key(IdentityKind::DataRoot, "_ROOT::A#B").is_err());
        assert!(validate_key(IdentityKind::Instance, "_ROOT::MAIN/R1").is_err());
        assert!(validate_key(IdentityKind::Task, "_ROOT::MAIN#FAST").is_err());
        assert!(validate_key(IdentityKind::Type, "_root::MAIN").is_err());
        assert!(validate_key(IdentityKind::Export, "_ROOT::F@()->VOID#C:1x").is_err());
    }

    #[test]
    fn hidden_root_keys_are_marked_and_distinct() {
        let global = hidden_root_key("_root", "main.motor", None, "HANDLE").unwrap();
        let instance =
            hidden_root_key("_root", "main.motor", Some("main/r1/inst"), "HANDLE").unwrap();
        let record =
            hidden_root_key("_root", "main.motor", Some("main/r1/inst"), "OBJREC").unwrap();
        assert_eq!(hidden_root_marker(&global), Some("HANDLE"));
        assert_eq!(hidden_root_marker(&instance), Some("HANDLE"));
        assert_eq!(hidden_root_marker(&record), Some("OBJREC"));
        assert_ne!(global, instance);
        assert_ne!(instance, record);
        assert_eq!(hidden_root_marker("_ROOT::MAIN.VALUE#MAIN/R1/INST"), None);
        assert_eq!(hidden_root_marker("_ROOT::MAIN/R1#FAST.GUARD"), None);
        assert!(hidden_root_key("_root", "main.motor", None, "HANDLES").is_err());
        // A field named HANDLE keeps its ordinary path key: the marker follows
        // the `#`, never a `.`.
        let field = field_root_key("_root", "main.handle", "main/r1/inst").unwrap();
        assert_eq!(hidden_root_marker(&field), None);
        assert_ne!(field, instance);
    }

    #[test]
    fn hidden_root_keys_require_a_complete_instance_path() {
        assert!(validate_key(IdentityKind::DataRoot, "_ROOT::MAIN.MOTOR#HANDLE").is_ok());
        assert!(
            validate_key(
                IdentityKind::DataRoot,
                "_ROOT::MAIN.MOTOR#MAIN/R1/INST/HANDLE"
            )
            .is_ok()
        );
        assert!(
            validate_key(
                IdentityKind::DataRoot,
                "_ROOT::MAIN.ARR.E0#MAIN/R1/INST/FB1/OBJREC"
            )
            .is_ok()
        );
        // A marker with a short instance path is not a hidden root.
        assert!(validate_key(IdentityKind::DataRoot, "_ROOT::MAIN.MOTOR#HANDLE/HANDLE").is_err());
        assert!(validate_key(IdentityKind::DataRoot, "_ROOT::MAIN.MOTOR#MAIN/HANDLE").is_err());
        // A three-segment instance named HANDLE keeps its ordinary root key.
        let ordinary = "_ROOT::MAIN.MOTOR#C/R/HANDLE";
        assert_eq!(hidden_root_marker(ordinary), None);
        assert!(validate_key(IdentityKind::DataRoot, ordinary).is_ok());
        // A nested instance whose final segment is a marker is reserved.
        assert_eq!(
            hidden_root_marker("_ROOT::MAIN.MOTOR#C/R/INST/NESTED/HANDLE"),
            Some("HANDLE")
        );
        // Ordinary roots still reject unknown suffixes.
        assert!(validate_key(IdentityKind::DataRoot, "_ROOT::MAIN.MOTOR#OTHER").is_err());
    }

    #[test]
    fn identity_tags_round_trip() {
        for tag in 1..=9 {
            let kind = IdentityKind::from_tag(tag).unwrap();
            assert_eq!(kind.as_u8(), tag);
        }
        assert!(IdentityKind::from_tag(0).is_none());
        assert!(IdentityKind::from_tag(10).is_none());
    }
}
