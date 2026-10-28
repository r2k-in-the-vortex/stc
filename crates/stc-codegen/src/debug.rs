//! Deterministic LLVM debug metadata (DWARF and CodeView).
//!
//! The metadata graph is built from real HIR spans and the canonical type
//! system. Node numbers are allocated monotonically, every map that can affect
//! output is ordered, and file names are package-relative with forward slashes.
//! Nothing here embeds the build host path, the current time, or a hash-map
//! iteration order.
//!
//! Debug metadata is emitted only when [`crate::Options::debug`] is true and a
//! [`stc_span::SourceMap`] is supplied. Function bodies, parameters, locals,
//! user types, global variables, and program fields are described. Persistent
//! roots live in host-owned GOT storage, so their `DIGlobalVariable` nodes are
//! declaration-only (`isDefinition: false`): they carry a stable source name and
//! type without inventing a module location.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use stc_got::Target;
use stc_hir::{DefId, DefKind, LocalId, ParamId, StringKind, TyId};
use stc_span::{FileId, SourceMap, Span};

use crate::CodegenError;
use crate::Options;
use crate::expr::BodyInfo;
use crate::ir::{Function, Value, quote};
use crate::layout::ScalarKind;
use crate::types::{NamedKind, NamedType, TypeCategory};
use crate::unit::Lowerer;

type Res<T> = Result<T, CodegenError>;

/// CodeView is the native debug format only for Windows MSVC targets; Windows
/// GNU targets use DWARF.
fn is_codeview_target(triple: &str) -> bool {
    triple.contains("windows") && triple.contains("msvc")
}

/// The debug metadata graph for one module.
pub(crate) struct DebugInfo<'a> {
    source_map: &'a SourceMap,
    /// Normalized package-relative path per `FileId`.
    paths: Vec<Option<String>>,
    /// CodeView on Windows MSVC targets, DWARF on every other target.
    codeview: bool,
    pointer_bits: u32,
    nodes: Vec<Option<String>>,
    cu: u32,
    primary_file: u32,
    module_flags: Vec<u32>,
    files: BTreeMap<FileId, u32>,
    type_nodes: BTreeMap<TyId, u32>,
    instance_nodes: BTreeMap<DefId, u32>,
    basic_types: BTreeMap<ScalarKind, u32>,
    void_type: Option<u32>,
    locations: BTreeMap<(u32, u32, u32, u32), u32>,
    param_variables: BTreeMap<ParamId, u32>,
    local_variables: BTreeMap<LocalId, u32>,
    retained: Vec<u32>,
    retained_seen: BTreeSet<u32>,
    globals: Vec<u32>,
    global_seen: BTreeSet<String>,
    bounds_type: Option<u32>,
    finalized: bool,
}

impl<'a> DebugInfo<'a> {
    /// Builds the fixed metadata skeleton, or returns `None` when debug output
    /// is disabled (including every release build) or no source text is
    /// available.
    pub(crate) fn new(options: &'a Options<'a>, target: &Target) -> Option<Self> {
        if !options.debug || options.release {
            return None;
        }
        let source_map = options.source_map?;
        if source_map.is_empty() {
            return None;
        }
        let paths = debug_paths(options, source_map);
        let primary_index = paths.iter().position(Option::is_some);
        let primary_path = paths
            .iter()
            .find_map(|path| path.clone())
            .or_else(|| normalize_path(options.artifact_name))
            .unwrap_or_else(|| "stc".into());
        let mut debug = Self {
            source_map,
            paths,
            codeview: is_codeview_target(&target.triple),
            pointer_bits: u32::from(target.pointer_width) * 8,
            nodes: Vec::new(),
            cu: 0,
            primary_file: 0,
            module_flags: Vec::new(),
            files: BTreeMap::new(),
            type_nodes: BTreeMap::new(),
            instance_nodes: BTreeMap::new(),
            basic_types: BTreeMap::new(),
            void_type: None,
            locations: BTreeMap::new(),
            param_variables: BTreeMap::new(),
            local_variables: BTreeMap::new(),
            retained: Vec::new(),
            retained_seen: BTreeSet::new(),
            globals: Vec::new(),
            global_seen: BTreeSet::new(),
            bounds_type: None,
            finalized: false,
        };
        debug.cu = debug.reserve();
        debug.primary_file = debug.alloc(format!(
            "!DIFile(filename: {}, directory: \".\")",
            quote(&primary_path)
        ));
        if let Some(index) = primary_index
            && let Ok(index) = u32::try_from(index)
        {
            debug.files.insert(FileId::new(index), debug.primary_file);
        }
        debug.add_module_flags();
        Some(debug)
    }

    fn add_module_flags(&mut self) {
        let ids = if self.codeview {
            [
                self.alloc("!{i32 2, !\"Debug Info Version\", i32 3}"),
                self.alloc("!{i32 1, !\"wchar_size\", i32 2}"),
                self.alloc("!{i32 2, !\"CodeView\", i32 1}"),
            ]
            .to_vec()
        } else {
            [
                self.alloc("!{i32 7, !\"Dwarf Version\", i32 5}"),
                self.alloc("!{i32 2, !\"Debug Info Version\", i32 3}"),
            ]
            .to_vec()
        };
        self.module_flags.extend(ids);
    }

    fn reserve(&mut self) -> u32 {
        let id = u32::try_from(self.nodes.len()).expect("debug metadata node limit");
        self.nodes.push(None);
        id
    }

    fn alloc(&mut self, body: impl Into<String>) -> u32 {
        let id = self.reserve();
        self.set_node(id, body);
        id
    }

    fn set_node(&mut self, id: u32, body: impl Into<String>) {
        let slot = self
            .nodes
            .get_mut(id as usize)
            .unwrap_or_else(|| panic!("debug metadata node {id} does not exist"));
        assert!(slot.is_none(), "debug metadata node {id} is already set");
        *slot = Some(body.into());
    }

    fn type_node(&self, ty: TyId) -> Option<u32> {
        self.type_nodes.get(&ty).copied()
    }

    fn reserve_type(&mut self, ty: TyId) -> u32 {
        let id = self.reserve();
        assert!(
            self.type_nodes.insert(ty, id).is_none(),
            "debug type {ty:?} is already reserved"
        );
        id
    }

    fn instance_node(&self, def: DefId) -> Option<u32> {
        self.instance_nodes.get(&def).copied()
    }

    fn reserve_instance(&mut self, def: DefId) -> u32 {
        let id = self.reserve();
        assert!(
            self.instance_nodes.insert(def, id).is_none(),
            "debug instance {def:?} is already reserved"
        );
        id
    }

    fn set_instance_node(&mut self, def: DefId, id: u32, body: impl Into<String>) {
        assert_eq!(self.instance_nodes.get(&def), Some(&id));
        self.set_node(id, body);
    }

    fn file_node(&mut self, file: FileId) -> Option<u32> {
        if let Some(id) = self.files.get(&file) {
            return Some(*id);
        }
        let path = self.paths.get(file.index()).cloned().flatten()?;
        let id = self.alloc(format!(
            "!DIFile(filename: {}, directory: \".\")",
            quote(&path)
        ));
        self.files.insert(file, id);
        Some(id)
    }

    /// Returns the `(DIFile node, line)` pair for a real source span.
    ///
    /// Synthetic spans and files without a normalized path produce no location,
    /// so a fabricated line 0 or host-absolute path can never reach metadata.
    fn span_file_line(&mut self, span: Span) -> Option<(u32, u32)> {
        if !is_real_span(span) {
            return None;
        }
        let file = self.file_node(span.file)?;
        let location = self.source_map.location(span);
        if location.line == 0 || location.column == 0 {
            return None;
        }
        Some((file, location.line))
    }

    /// Adds a `DILocation` in `scope` for a real source span.
    fn location(&mut self, span: Span, scope: u32) -> Option<u32> {
        if !is_real_span(span) {
            return None;
        }
        let (file, line) = self.span_file_line(span)?;
        let location = self.source_map.location(span);
        if location.column == 0 {
            return None;
        }
        let key = (scope, file, line, location.column);
        if let Some(id) = self.locations.get(&key) {
            return Some(*id);
        }
        let id = self.alloc(format!(
            "!DILocation(line: {line}, column: {}, scope: !{scope})",
            location.column
        ));
        self.locations.insert(key, id);
        Some(id)
    }

    fn basic_type(&mut self, kind: ScalarKind) -> u32 {
        if let Some(id) = self.basic_types.get(&kind) {
            return *id;
        }
        let name = kind.canonical_name();
        let size = kind.size_bytes() * 8;
        let encoding = match kind {
            ScalarKind::Bool => "DW_ATE_boolean",
            ScalarKind::Int { signed: true, .. } => "DW_ATE_signed",
            ScalarKind::Int { signed: false, .. } => "DW_ATE_unsigned",
            ScalarKind::Real { .. } => "DW_ATE_float",
            ScalarKind::Time | ScalarKind::Date(_) => "DW_ATE_signed",
            ScalarKind::Char { .. } => "DW_ATE_unsigned_char",
        };
        let id = self.alloc(format!(
            "!DIBasicType(name: {}, size: {size}, encoding: {encoding})",
            quote(&name)
        ));
        self.basic_types.insert(kind, id);
        id
    }

    fn void_type(&mut self) -> u32 {
        if let Some(id) = self.void_type {
            return id;
        }
        let id = self.alloc("!DIBasicType(tag: DW_TAG_unspecified_type, name: \"void\")");
        self.void_type = Some(id);
        id
    }

    fn raw_pointer(&mut self) -> u32 {
        let target = self.void_type();
        self.pointer_to(target)
    }

    fn pointer_body(&self, target: u32) -> String {
        format!(
            "!DIDerivedType(tag: DW_TAG_pointer_type, size: {}, align: {}, baseType: !{target})",
            self.pointer_bits, self.pointer_bits
        )
    }

    fn pointer_to(&mut self, target: u32) -> u32 {
        let body = self.pointer_body(target);
        self.alloc(body)
    }

    #[allow(clippy::too_many_arguments)]
    fn member(
        &mut self,
        scope: u32,
        name: &str,
        file_line: Option<(u32, u32)>,
        base_type: u32,
        offset: u64,
        size: u64,
        align: u32,
    ) -> u32 {
        let mut fields = vec![
            "tag: DW_TAG_member".to_owned(),
            format!("name: {}", quote(name)),
            format!("scope: !{scope}"),
        ];
        if let Some((file, line)) = file_line {
            fields.push(format!("file: !{file}"));
            fields.push(format!("line: {line}"));
        }
        fields.push(format!("baseType: !{base_type}"));
        if size > 0 {
            fields.push(format!("size: {size}"));
        }
        if align > 0 {
            fields.push(format!("align: {align}"));
        }
        if offset > 0 {
            fields.push(format!("offset: {offset}"));
        }
        self.alloc(format!("!DIDerivedType({})", fields.join(", ")))
    }

    fn inheritance(&mut self, scope: u32, base_type: u32) -> u32 {
        self.alloc(format!(
            "!DIDerivedType(tag: DW_TAG_inheritance, scope: !{scope}, baseType: !{base_type}, \
             offset: 0, flags: DIFlagPublic)"
        ))
    }

    fn enumerator(&mut self, name: &str, value: i128) -> u32 {
        self.alloc(format!(
            "!DIEnumerator(name: {}, value: {value})",
            quote(name)
        ))
    }

    fn subrange(&mut self, length: u64, lower: i128) -> u32 {
        let mut fields = vec![format!("count: {length}")];
        if lower != 0 {
            fields.push(format!("lowerBound: {lower}"));
        }
        self.alloc(format!("!DISubrange({})", fields.join(", ")))
    }

    fn tuple(&mut self, ids: &[u32]) -> u32 {
        let entries = ids
            .iter()
            .map(|id| format!("!{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        self.alloc(format!("!{{{entries}}}"))
    }

    #[allow(clippy::too_many_arguments)]
    fn composite(
        &mut self,
        tag: &str,
        name: Option<&str>,
        file_line: Option<(u32, u32)>,
        identifier: Option<&str>,
        base_type: Option<u32>,
        size: u64,
        align: u32,
        elements: &[u32],
    ) -> String {
        let mut fields = vec![format!("tag: {tag}")];
        if let Some(name) = name {
            fields.push(format!("name: {}", quote(name)));
        }
        if let Some((file, line)) = file_line {
            fields.push(format!("scope: !{file}"));
            fields.push(format!("file: !{file}"));
            fields.push(format!("line: {line}"));
        }
        if let Some(identifier) = identifier {
            fields.push(format!("identifier: {}", quote(identifier)));
        }
        if let Some(base_type) = base_type {
            fields.push(format!("baseType: !{base_type}"));
        }
        if size > 0 {
            fields.push(format!("size: {size}"));
        }
        if align > 0 {
            fields.push(format!("align: {align}"));
        }
        if !elements.is_empty() {
            let tuple = self.tuple(elements);
            fields.push(format!("elements: !{tuple}"));
        }
        format!("distinct !DICompositeType({})", fields.join(", "))
    }

    fn subroutine_type(&mut self, result: Option<u32>, params: &[u32]) -> u32 {
        let mut entries = Vec::with_capacity(params.len() + 1);
        entries.push(result.map_or_else(|| "null".to_owned(), |id| format!("!{id}")));
        entries.extend(params.iter().map(|id| format!("!{id}")));
        let tuple = self.alloc(format!("!{{{}}}", entries.join(", ")));
        self.alloc(format!("!DISubroutineType(types: !{tuple})"))
    }

    fn local_variable(
        &mut self,
        name: &str,
        scope: u32,
        file_line: (u32, u32),
        ty: u32,
        arg: Option<u32>,
    ) -> u32 {
        let (file, line) = file_line;
        let argument = arg.map(|arg| format!(", arg: {arg}")).unwrap_or_default();
        self.alloc(format!(
            "!DILocalVariable(name: {}, scope: !{scope}, file: !{file}, line: {line}, \
             type: !{ty}{argument})",
            quote(name)
        ))
    }

    fn param_variable(&self, param: ParamId) -> Option<u32> {
        self.param_variables.get(&param).copied()
    }

    fn local_variable_id(&self, local: LocalId) -> Option<u32> {
        self.local_variables.get(&local).copied()
    }

    fn add_retained(&mut self, id: u32) {
        if self.retained_seen.insert(id) {
            self.retained.push(id);
        }
    }

    /// Adds a declaration-only global expression.
    fn add_global(&mut self, name: &str, linkage: Option<&str>, ty: u32, span: Span) {
        let Some((file, line)) = self.span_file_line(span) else {
            return;
        };
        let key = format!("{name}|{file}|{line}|{ty}");
        if !self.global_seen.insert(key) {
            return;
        }
        let linkage = linkage.map_or_else(String::new, |linkage| {
            format!("linkageName: {}, ", quote(linkage))
        });
        let variable = self.alloc(format!(
            "!DIGlobalVariable(name: {}, {linkage}scope: !{}, file: !{file}, line: {line}, \
             type: !{ty}, isLocal: false, isDefinition: false)",
            quote(name),
            self.cu
        ));
        let expression = self.alloc(format!(
            "!DIGlobalVariableExpression(var: !{variable}, expr: !DIExpression())"
        ));
        self.globals.push(expression);
    }

    fn bounds_type(&mut self) -> u32 {
        if let Some(id) = self.bounds_type {
            return id;
        }
        let i64_ty = self.basic_type(ScalarKind::Int {
            signed: true,
            width: 64,
        });
        let reserved = self.reserve();
        let lower = self.member(reserved, "lower", None, i64_ty, 0, 64, 64);
        let upper = self.member(reserved, "upper", None, i64_ty, 64, 64, 64);
        let body = self.composite(
            "DW_TAG_structure_type",
            Some("ARRAY_BOUNDS"),
            None,
            None,
            None,
            128,
            64,
            &[lower, upper],
        );
        self.set_node(reserved, body);
        self.bounds_type = Some(reserved);
        reserved
    }

    fn finalize(&mut self, producer: &str) -> Res<()> {
        if self.finalized {
            return Ok(());
        }
        let mut fields = vec![
            "language: DW_LANG_C99".to_owned(),
            format!("file: !{}", self.primary_file),
            format!("producer: {}", quote(producer)),
            "isOptimized: false".to_owned(),
            "runtimeVersion: 0".to_owned(),
            "emissionKind: FullDebug".to_owned(),
        ];
        if !self.globals.is_empty() {
            let globals = self.globals.clone();
            let tuple = self.tuple(&globals);
            fields.push(format!("globals: !{tuple}"));
        }
        if !self.retained.is_empty() {
            let retained = self.retained.clone();
            let tuple = self.tuple(&retained);
            fields.push(format!("retainedTypes: !{tuple}"));
        }
        let body = format!("distinct !DICompileUnit({})", fields.join(", "));
        self.set_node(self.cu, body);
        self.finalized = true;
        Ok(())
    }

    fn render(&self) -> Res<String> {
        let mut out = String::new();
        let _ = writeln!(out, "!llvm.dbg.cu = !{{!{}}}", self.cu);
        let flags = self
            .module_flags
            .iter()
            .map(|id| format!("!{id}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "!llvm.module.flags = !{{{flags}}}");
        out.push('\n');
        for (index, node) in self.nodes.iter().enumerate() {
            let Some(body) = node else {
                return Err(CodegenError::internal(format!(
                    "debug metadata node {index} has no body"
                )));
            };
            let _ = writeln!(out, "!{index} = {body}");
        }
        Ok(out)
    }
}

impl Lowerer<'_> {
    pub(crate) fn debug_enabled(&self) -> bool {
        self.debug.is_some()
    }

    fn debug_span_file_line(&mut self, span: Span) -> Option<(u32, u32)> {
        self.debug.as_mut()?.span_file_line(span)
    }

    /// Sets the current instruction location from a real source span.
    pub(crate) fn set_debug_location(&mut self, f: &mut Function, span: Span) {
        let Some(scope) = f.debug_subprogram() else {
            return;
        };
        let location = self
            .debug
            .as_mut()
            .and_then(|debug| debug.location(span, scope));
        if let Some(location) = location {
            f.set_debug_location(Some(location));
        }
    }

    fn debug_def_file_line(&mut self, def: DefId) -> Option<(u32, u32)> {
        let span = self.hir.defs[def.index()].span;
        self.debug_span_file_line(span)
    }

    fn debug_type_required(&mut self, ty: TyId) -> Res<u32> {
        self.debug_type(ty)?.ok_or_else(|| {
            CodegenError::internal("debug type requested while debug emission is disabled")
        })
    }

    /// Returns the debug type node for a canonical type, creating the type
    /// graph on first use.
    fn debug_type(&mut self, ty: TyId) -> Res<Option<u32>> {
        if !self.debug_enabled() {
            return Ok(None);
        }
        if let Some(id) = self.debug.as_ref().and_then(|debug| debug.type_node(ty)) {
            return Ok(Some(id));
        }
        let facts = self.types.facts(ty)?.clone();
        let node = match facts.category {
            TypeCategory::Scalar(kind) => self
                .debug
                .as_mut()
                .expect("debug is enabled")
                .basic_type(kind),
            TypeCategory::Null => self.debug.as_mut().expect("debug is enabled").raw_pointer(),
            TypeCategory::Object { def } if facts.class_value => {
                let instance = self.debug_instance_type(def)?;
                self.debug
                    .as_mut()
                    .expect("debug is enabled")
                    .pointer_to(instance)
            }
            TypeCategory::Object { def }
            | TypeCategory::Struct { def, .. }
            | TypeCategory::Interface { def }
            | TypeCategory::Enum { def, .. }
            | TypeCategory::Subrange { def, .. }
            | TypeCategory::NamedValue { def, .. } => self.debug_instance_type(def)?,
            TypeCategory::Pointer { target } => {
                let reserved = self
                    .debug
                    .as_mut()
                    .expect("debug is enabled")
                    .reserve_type(ty);
                let target = self.debug_type_required(target)?;
                let body = self
                    .debug
                    .as_ref()
                    .expect("debug is enabled")
                    .pointer_body(target);
                self.debug
                    .as_mut()
                    .expect("debug is enabled")
                    .set_node(reserved, body);
                reserved
            }
            TypeCategory::Array {
                element,
                dims,
                conformant,
            } => {
                let reserved = self
                    .debug
                    .as_mut()
                    .expect("debug is enabled")
                    .reserve_type(ty);
                let body = if conformant {
                    self.debug_conformant_array(
                        reserved,
                        element,
                        dims.len(),
                        facts.layout.size,
                        facts.layout.align,
                    )?
                } else {
                    self.debug_static_array(element, &dims, facts.layout.size, facts.layout.align)?
                };
                self.debug
                    .as_mut()
                    .expect("debug is enabled")
                    .set_node(reserved, body);
                reserved
            }
            TypeCategory::String {
                kind,
                capacity,
                unit_bytes,
            } => {
                let reserved = self
                    .debug
                    .as_mut()
                    .expect("debug is enabled")
                    .reserve_type(ty);
                let body = self.debug_string(
                    reserved,
                    kind,
                    capacity,
                    unit_bytes,
                    facts.layout.size,
                    facts.layout.align,
                )?;
                self.debug
                    .as_mut()
                    .expect("debug is enabled")
                    .set_node(reserved, body);
                reserved
            }
            TypeCategory::Reference { target } => {
                let reserved = self
                    .debug
                    .as_mut()
                    .expect("debug is enabled")
                    .reserve_type(ty);
                let target = self.debug_reference_target(target)?;
                let body =
                    self.debug_reference(reserved, facts.layout.size, facts.layout.align, target)?;
                self.debug
                    .as_mut()
                    .expect("debug is enabled")
                    .set_node(reserved, body);
                reserved
            }
        };
        self.debug
            .as_mut()
            .expect("debug is enabled")
            .type_nodes
            .insert(ty, node);
        Ok(Some(node))
    }

    fn debug_reference_target(&mut self, target: TyId) -> Res<u32> {
        let facts = self.types.facts(target)?.clone();
        if facts.class_value
            && let TypeCategory::Object { def } = facts.category
        {
            return self.debug_instance_type(def);
        }
        self.debug_type_required(target)
    }

    fn debug_static_array(
        &mut self,
        element: TyId,
        dims: &[crate::types::ArrayDim],
        size: u64,
        align: u32,
    ) -> Res<String> {
        let base = self.debug_type_required(element)?;
        let mut elements = Vec::with_capacity(dims.len());
        for dim in dims {
            let subrange = self
                .debug
                .as_mut()
                .expect("debug is enabled")
                .subrange(dim.length, dim.lower);
            elements.push(subrange);
        }
        let Some(debug) = self.debug.as_mut() else {
            return Ok(String::new());
        };
        Ok(debug.composite(
            "DW_TAG_array_type",
            None,
            None,
            None,
            Some(base),
            size * 8,
            align * 8,
            &elements,
        ))
    }

    fn debug_conformant_array(
        &mut self,
        reserved: u32,
        element: TyId,
        rank: usize,
        size: u64,
        align: u32,
    ) -> Res<String> {
        let element = self.debug_type_required(element)?;
        let pointer_bits = self.debug.as_ref().map_or(64, |debug| debug.pointer_bits);
        let bounds = self.debug.as_mut().expect("debug is enabled").bounds_type();
        let count = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .subrange(u64::try_from(rank).unwrap_or(u64::MAX), 0);
        let rank_bits = u64::try_from(rank).unwrap_or(0) * 128;
        let bounds_array = {
            let Some(debug) = self.debug.as_mut() else {
                return Ok(String::new());
            };
            debug.composite(
                "DW_TAG_array_type",
                None,
                None,
                None,
                Some(bounds),
                rank_bits,
                64,
                &[count],
            )
        };
        let pointer = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .pointer_to(element);
        let total = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .basic_type(ScalarKind::Int {
                signed: true,
                width: 64,
            });
        let data_member = self.debug.as_mut().expect("debug is enabled").member(
            reserved,
            "data",
            None,
            pointer,
            0,
            u64::from(pointer_bits),
            pointer_bits,
        );
        let total_member = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .member(reserved, "total", None, total, 64, 64, 64);
        let bounds_array = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .alloc(bounds_array);
        let bounds_member = self.debug.as_mut().expect("debug is enabled").member(
            reserved,
            "bounds",
            None,
            bounds_array,
            128,
            rank_bits,
            64,
        );
        let Some(debug) = self.debug.as_mut() else {
            return Ok(String::new());
        };
        Ok(debug.composite(
            "DW_TAG_structure_type",
            Some("ARRAY[*]"),
            None,
            None,
            None,
            size * 8,
            align * 8,
            &[data_member, total_member, bounds_member],
        ))
    }

    fn debug_string(
        &mut self,
        reserved: u32,
        kind: StringKind,
        capacity: u32,
        unit_bytes: u8,
        size: u64,
        align: u32,
    ) -> Res<String> {
        let name = match kind {
            StringKind::Sbcs => "STRING",
            StringKind::Dbcs => "WSTRING",
            StringKind::Unicode => "USTRING",
        };
        let length = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .basic_type(ScalarKind::Int {
                signed: true,
                width: 32,
            });
        let char_ty = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .basic_type(ScalarKind::Char {
                width: u16::from(unit_bytes) * 8,
            });
        let count = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .subrange(u64::from(capacity), 0);
        let data_bits = u64::from(capacity) * u64::from(unit_bytes) * 8;
        let unit_bits = u32::from(unit_bytes) * 8;
        let data_type = {
            let Some(debug) = self.debug.as_mut() else {
                return Ok(String::new());
            };
            debug.composite(
                "DW_TAG_array_type",
                None,
                None,
                None,
                Some(char_ty),
                data_bits,
                unit_bits,
                &[count],
            )
        };
        let length_member = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .member(reserved, "length", None, length, 0, 32, 32);
        let data_member = {
            let Some(debug) = self.debug.as_mut() else {
                return Ok(String::new());
            };
            let data = debug.alloc(data_type);
            debug.member(reserved, "data", None, data, 32, data_bits, unit_bits)
        };
        let Some(debug) = self.debug.as_mut() else {
            return Ok(String::new());
        };
        Ok(debug.composite(
            "DW_TAG_structure_type",
            Some(&format!("{name}[{capacity}]")),
            None,
            None,
            None,
            size * 8,
            align * 8,
            &[length_member, data_member],
        ))
    }

    fn debug_reference(
        &mut self,
        reserved: u32,
        size: u64,
        align: u32,
        target: u32,
    ) -> Res<String> {
        let pointer_bits = self.debug.as_ref().map_or(64, |debug| debug.pointer_bits);
        let i32_ty = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .basic_type(ScalarKind::Int {
                signed: true,
                width: 32,
            });
        let i64_ty = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .basic_type(ScalarKind::Int {
                signed: true,
                width: 64,
            });
        let pointer = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .pointer_to(target);
        let mut members = Vec::new();
        for (name, ty, offset, bits) in [
            ("kind", i32_ty, 0_u64, 32_u64),
            ("flags", i32_ty, 32, 32),
            ("offset", i64_ty, 64, 64),
            ("id_lo", i64_ty, 128, 64),
            ("id_hi", i64_ty, 192, 64),
            ("data", pointer, 256, u64::from(pointer_bits)),
            ("typeinfo", pointer, 320, u64::from(pointer_bits)),
        ] {
            let member = self.debug.as_mut().expect("debug is enabled").member(
                reserved,
                name,
                None,
                ty,
                offset,
                bits,
                u32::try_from(bits).unwrap_or(32),
            );
            members.push(member);
        }
        let Some(debug) = self.debug.as_mut() else {
            return Ok(String::new());
        };
        Ok(debug.composite(
            "DW_TAG_structure_type",
            Some("REFERENCE"),
            None,
            None,
            None,
            size * 8,
            align * 8,
            &members,
        ))
    }

    fn debug_instance_type(&mut self, def: DefId) -> Res<u32> {
        if let Some(id) = self
            .debug
            .as_ref()
            .and_then(|debug| debug.instance_node(def))
        {
            return Ok(id);
        }
        let reserved = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .reserve_instance(def);
        let named = self.types.named(def)?.clone();
        let body = self.debug_named_type(def, &named)?;
        self.debug
            .as_mut()
            .expect("debug is enabled")
            .set_instance_node(def, reserved, body);
        Ok(reserved)
    }

    fn debug_named_type(&mut self, def: DefId, named: &NamedType) -> Res<String> {
        let file_line = self.debug_def_file_line(def);
        let identifier = format!("stc:T:{}", named.id.to_hex_plain());
        let qualified = self.qualified_path(def);
        match named.kind {
            NamedKind::Struct | NamedKind::Class | NamedKind::FunctionBlock => {
                let tag = if named.kind == NamedKind::Class {
                    "DW_TAG_class_type"
                } else {
                    "DW_TAG_structure_type"
                };
                let layout = if named.kind.is_object() {
                    let ty = self.types.named_ty(def)?;
                    self.types.instance_layout(ty)?
                } else {
                    named.layout
                };
                // Reserve the composite before members so recursive types and
                // inheritance can refer to it.
                let reserved = self
                    .debug
                    .as_ref()
                    .and_then(|debug| debug.instance_node(def))
                    .expect("instance type is reserved");
                let mut elements = Vec::new();
                if let Some(base) = named.base {
                    let base = self.debug_instance_type(base)?;
                    elements.push(
                        self.debug
                            .as_mut()
                            .expect("debug is enabled")
                            .inheritance(reserved, base),
                    );
                }
                for field in &named.fields {
                    let field_data = self.hir.field(field.field).clone();
                    let ty = self.debug_type_required(field.ty)?;
                    let field_line = self.debug_span_file_line(field_data.span);
                    let name = self.hir.interner.text(field_data.name).to_string();
                    let member = self.debug.as_mut().expect("debug is enabled").member(
                        reserved,
                        &name,
                        field_line,
                        ty,
                        field.offset * 8,
                        field.layout.size * 8,
                        field.layout.align * 8,
                    );
                    elements.push(member);
                }
                let Some(debug) = self.debug.as_mut() else {
                    return Ok(String::new());
                };
                Ok(debug.composite(
                    tag,
                    Some(&qualified),
                    file_line,
                    Some(&identifier),
                    None,
                    layout.size * 8,
                    layout.align * 8,
                    &elements,
                ))
            }
            NamedKind::Interface => {
                let layout = named.layout;
                let reserved = self
                    .debug
                    .as_ref()
                    .and_then(|debug| debug.instance_node(def))
                    .expect("interface type is reserved");
                let pointer_bits = u32::from(self.target.pointer_width) * 8;
                let record = self.debug.as_mut().expect("debug is enabled").raw_pointer();
                let vtable = self.debug.as_mut().expect("debug is enabled").raw_pointer();
                let record = self.debug.as_mut().expect("debug is enabled").member(
                    reserved,
                    "record",
                    file_line,
                    record,
                    0,
                    u64::from(pointer_bits),
                    pointer_bits,
                );
                let vtable = self.debug.as_mut().expect("debug is enabled").member(
                    reserved,
                    "vtable",
                    file_line,
                    vtable,
                    u64::from(pointer_bits),
                    u64::from(pointer_bits),
                    pointer_bits,
                );
                let Some(debug) = self.debug.as_mut() else {
                    return Ok(String::new());
                };
                Ok(debug.composite(
                    "DW_TAG_structure_type",
                    Some(&qualified),
                    file_line,
                    Some(&identifier),
                    None,
                    layout.size * 8,
                    layout.align * 8,
                    &[record, vtable],
                ))
            }
            NamedKind::Enum | NamedKind::NamedValue => {
                let base_scalar = self
                    .types
                    .facts(self.types.named_ty(def)?)?
                    .scalar()
                    .ok_or_else(|| CodegenError::internal("enum has no scalar base"))?;
                let base = self
                    .debug
                    .as_mut()
                    .expect("debug is enabled")
                    .basic_type(base_scalar);
                let mut values = Vec::with_capacity(named.enum_values.len());
                for value in &named.enum_values {
                    let name = self.hir.interner.text(value.name).to_string();
                    values.push(
                        self.debug
                            .as_mut()
                            .expect("debug is enabled")
                            .enumerator(&name, value.value),
                    );
                }
                let Some(debug) = self.debug.as_mut() else {
                    return Ok(String::new());
                };
                Ok(debug.composite(
                    "DW_TAG_enumeration_type",
                    Some(&qualified),
                    file_line,
                    Some(&identifier),
                    Some(base),
                    named.layout.size * 8,
                    named.layout.align * 8,
                    &values,
                ))
            }
            NamedKind::Subrange => {
                let base_scalar = self
                    .types
                    .facts(self.types.named_ty(def)?)?
                    .scalar()
                    .ok_or_else(|| CodegenError::internal("subrange has no scalar base"))?;
                let base = self
                    .debug
                    .as_mut()
                    .expect("debug is enabled")
                    .basic_type(base_scalar);
                let mut fields = vec![
                    "tag: DW_TAG_typedef".to_owned(),
                    format!("name: {}", quote(&qualified)),
                ];
                if let Some((file, line)) = file_line {
                    fields.push(format!("scope: !{file}"));
                    fields.push(format!("file: !{file}"));
                    fields.push(format!("line: {line}"));
                }
                fields.push(format!("baseType: !{base}"));
                if named.layout.size > 0 {
                    fields.push(format!("size: {}", named.layout.size * 8));
                }
                if named.layout.align > 0 {
                    fields.push(format!("align: {}", named.layout.align * 8));
                }
                Ok(format!("!DIDerivedType({})", fields.join(", ")))
            }
        }
    }

    /// Creates the subprogram signature and local-variable metadata for a user
    /// function body. The function's source definition must be real; synthetic
    /// compiler bodies are intentionally not described.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_debug_function(
        &mut self,
        f: &mut Function,
        def: DefId,
        name: String,
        result: Option<TyId>,
        params: &[ParamId],
        locals: &[LocalId],
        body_span: Span,
    ) -> Res<()> {
        if !self.debug_enabled() {
            return Ok(());
        }
        let def_span = self.hir.defs[def.index()].span;
        if !is_real_span(def_span) {
            return Ok(());
        }
        let Some((def_file, def_line)) = self.debug_def_file_line(def) else {
            return Ok(());
        };
        if self.checkpoint_collection_enabled() {
            // The frame sidecar needs a reliable saved-frame-pointer chain.
            f.set_debug_frames();
        }
        let scope_line = self
            .debug_span_file_line(body_span)
            .map_or(def_line, |(_, line)| line);
        let result_ty = match result {
            Some(ty) => Some(self.debug_type_required(ty)?),
            None => None,
        };
        let mut param_types = Vec::with_capacity(params.len());
        for param in params {
            param_types.push(self.debug_type_required(self.hir.param(*param).ty)?);
        }
        let signature = self
            .debug
            .as_mut()
            .expect("debug is enabled")
            .subroutine_type(result_ty, &param_types);
        let subprogram = self.debug.as_mut().expect("debug is enabled").reserve();
        let mut variable_ids = Vec::new();
        let mut frame_locals = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let data = self.hir.param(*param).clone();
            let Some(type_id) = self.debug_type(data.ty)? else {
                continue;
            };
            let Some(file_line) = self.debug_span_file_line(data.span) else {
                continue;
            };
            let name = self.hir.interner.text(data.name).to_string();
            if self.checkpoint_collection_enabled() {
                self.push_frame_hint(&mut frame_locals, &name, data.ty, true, data.span)?;
            }
            let arg = u32::try_from(index).ok().map(|index| index + 1);
            let variable = self
                .debug
                .as_mut()
                .expect("debug is enabled")
                .local_variable(&name, subprogram, file_line, type_id, arg);
            self.debug
                .as_mut()
                .expect("debug is enabled")
                .param_variables
                .insert(*param, variable);
            variable_ids.push(variable);
        }
        for local in locals {
            let data = self.hir.local(*local).clone();
            if data.external_target.is_some() {
                continue;
            }
            let Some(type_id) = self.debug_type(data.ty)? else {
                continue;
            };
            let Some(file_line) = self.debug_span_file_line(data.span) else {
                continue;
            };
            let name = self.hir.interner.text(data.name).to_string();
            if self.checkpoint_collection_enabled() {
                self.push_frame_hint(&mut frame_locals, &name, data.ty, false, data.span)?;
            }
            let variable = self
                .debug
                .as_mut()
                .expect("debug is enabled")
                .local_variable(&name, subprogram, file_line, type_id, None);
            self.debug
                .as_mut()
                .expect("debug is enabled")
                .local_variables
                .insert(*local, variable);
            variable_ids.push(variable);
        }
        let retained = if variable_ids.is_empty() {
            None
        } else {
            let Some(debug) = self.debug.as_mut() else {
                return Ok(());
            };
            Some(debug.tuple(&variable_ids))
        };
        let linkage = quote(&f.name);
        let retained =
            retained.map_or_else(String::new, |tuple| format!(", retainedNodes: !{tuple}"));
        let body = format!(
            "distinct !DISubprogram(name: {}, linkageName: {linkage}, scope: !{def_file}, \
             file: !{def_file}, line: {def_line}, type: !{signature}, scopeLine: {scope_line}, \
             flags: DIFlagPrototyped, spFlags: DISPFlagDefinition, unit: !{}{retained})",
            quote(&name),
            self.debug.as_ref().map_or(0, |debug| debug.cu)
        );
        let Some(debug) = self.debug.as_mut() else {
            return Ok(());
        };
        debug.set_node(subprogram, body);
        f.set_debug_subprogram(Some(subprogram));
        let location = self
            .debug
            .as_mut()
            .and_then(|debug| debug.location(def_span, subprogram));
        f.set_debug_location(location);
        if self.checkpoint_collection_enabled() {
            let target = match &self.hir.defs[def.index()].kind {
                DefKind::Function { .. } => Some(crate::module::FunctionTarget::Function(def)),
                DefKind::FunctionBlock { .. } => Some(crate::module::FunctionTarget::FbBody(def)),
                DefKind::Method { .. } => Some(crate::module::FunctionTarget::Method(def)),
                DefKind::Program { .. } => Some(crate::module::FunctionTarget::ProgramBody(def)),
                _ => None,
            };
            if let Some(got_id) = target.and_then(|target| self.got.function_got_id(target)) {
                self.frame_hints.push(crate::FrameFunctionHint {
                    function: got_id,
                    symbol: f.name.clone(),
                    locals: frame_locals,
                });
            }
        }
        Ok(())
    }

    /// Records one front-end parameter or local hint for the frame sidecar.
    fn push_frame_hint(
        &self,
        hints: &mut Vec<crate::FrameLocalHint>,
        name: &str,
        ty: TyId,
        parameter: bool,
        span: Span,
    ) -> Res<()> {
        let facts = self.types.facts(ty)?;
        if facts.layout.size == 0 {
            return Ok(());
        }
        let size = u32::try_from(facts.layout.size).map_err(|_| {
            CodegenError::internal("a frame local exceeds the 32-bit byte-size limit")
        })?;
        hints.push(crate::FrameLocalHint {
            name: name.to_ascii_uppercase(),
            type_key: facts.key.clone(),
            size,
            parameter,
            declaration: span.start,
            first: 0,
            last: 0,
        });
        Ok(())
    }

    /// Emits `llvm.dbg.declare` for parameters that have a storage pointer.
    pub(crate) fn declare_debug_params(
        &mut self,
        f: &mut Function,
        params: &[ParamId],
        info: &BodyInfo,
    ) {
        if !self.debug_enabled() || f.debug_subprogram().is_none() {
            return;
        }
        let previous = f.debug_location();
        for param in params {
            let Some(variable) = self
                .debug
                .as_ref()
                .and_then(|debug| debug.param_variable(*param))
            else {
                continue;
            };
            let Some(storage) = info.params.get(param) else {
                continue;
            };
            let span = self.hir.param(*param).span;
            self.set_debug_location(f, span);
            f.debug_declare(variable, storage);
        }
        f.set_debug_location(previous);
    }

    /// Emits `llvm.dbg.declare` for one local after its alloca exists.
    pub(crate) fn declare_debug_local(
        &mut self,
        f: &mut Function,
        local: LocalId,
        storage: &Value,
    ) {
        if !self.debug_enabled() || f.debug_subprogram().is_none() {
            return;
        }
        let Some(variable) = self
            .debug
            .as_ref()
            .and_then(|debug| debug.local_variable_id(local))
        else {
            return;
        };
        let previous = f.debug_location();
        let span = self.hir.local(local).span;
        self.set_debug_location(f, span);
        f.debug_declare(variable, storage);
        f.set_debug_location(previous);
    }

    fn collect_debug_retained_types(&mut self) -> Res<()> {
        let defs: Vec<DefId> = self.types.named_types().map(|named| named.def).collect();
        for def in defs {
            let data = &self.hir.defs[def.index()];
            if (data.test_only && !self.options.test_build) || !is_real_span(data.span) {
                continue;
            }
            // Unused value declarations may not have an interned type in the
            // arena. Their layout carries no types that can be described
            // safely, so they are omitted rather than inventing a node.
            // Structs and interfaces can be described directly from the named
            // layout, so an unused declaration of either is still visible.
            let needs_arena = matches!(
                self.types.named(def)?.kind,
                NamedKind::FunctionBlock
                    | NamedKind::Class
                    | NamedKind::Enum
                    | NamedKind::NamedValue
                    | NamedKind::Subrange
            );
            if needs_arena && self.types.named_ty(def).is_err() {
                continue;
            }
            let node = self.debug_instance_type(def)?;
            self.debug
                .as_mut()
                .expect("debug is enabled")
                .add_retained(node);
        }
        Ok(())
    }

    fn collect_debug_globals(&mut self) -> Res<()> {
        let globals: Vec<(DefId, TyId, Span)> = self
            .hir
            .defs
            .iter()
            .enumerate()
            .filter_map(|(index, def)| {
                let id = DefId::new(u32::try_from(index).ok()?);
                match &def.kind {
                    DefKind::Global { ty, .. } => Some((id, *ty, def.span)),
                    _ => None,
                }
            })
            .collect();
        for (def, ty, span) in globals {
            let data = &self.hir.defs[def.index()];
            if (data.test_only && !self.options.test_build) || !is_real_span(span) {
                continue;
            }
            let Some(node) = self.debug_type(ty)? else {
                continue;
            };
            let name = self.qualified_path(def);
            let linkage = self.global_symbols.get(&def).cloned();
            self.debug.as_mut().expect("debug is enabled").add_global(
                &name,
                linkage.as_deref(),
                node,
                span,
            );
        }

        let programs = self.plan.programs.clone();
        for program in programs {
            let data = &self.hir.defs[program.index()];
            if (data.test_only && !self.options.test_build) || !is_real_span(data.span) {
                continue;
            }
            let DefKind::Program { fields, .. } = data.kind.clone() else {
                continue;
            };
            let program_name = self.qualified_path(program);
            for field in fields {
                let field_data = self.hir.field(field).clone();
                if !is_real_span(field_data.span) {
                    continue;
                }
                let Some(node) = self.debug_type(field_data.ty)? else {
                    continue;
                };
                let field_name = self.field_name(field);
                let name = format!("{program_name}.{field_name}");
                self.debug.as_mut().expect("debug is enabled").add_global(
                    &name,
                    None,
                    node,
                    field_data.span,
                );
            }
        }
        Ok(())
    }

    /// Finalizes the compile unit and attaches rendered metadata to the module.
    pub(crate) fn finalize_debug(&mut self) -> Res<()> {
        if !self.debug_enabled() {
            return Ok(());
        }
        self.collect_debug_retained_types()?;
        self.collect_debug_globals()?;
        let producer = format!("stc {}", env!("CARGO_PKG_VERSION"));
        let rendered = {
            let Some(debug) = self.debug.as_mut() else {
                return Ok(());
            };
            debug.finalize(&producer)?;
            debug.render()?
        };
        self.module.set_debug_metadata(rendered);
        Ok(())
    }
}

fn is_real_span(span: Span) -> bool {
    span.start < span.end
}

fn debug_paths(options: &Options<'_>, source_map: &SourceMap) -> Vec<Option<String>> {
    let relative = options
        .source_paths
        .is_none()
        .then(|| crate::module::package_relative_paths(options.sources));
    let mut paths = Vec::with_capacity(source_map.len());
    for index in 0..source_map.len() {
        let explicit = options.source_paths.and_then(|paths| paths.get(index));
        let from_sources = options.sources.get(index);
        let fallback = || {
            source_map
                .file_path(FileId::new(u32::try_from(index).unwrap_or(u32::MAX)))
                .and_then(normalize_path)
        };
        let path = explicit
            .and_then(|path| normalize_path(path))
            .or_else(|| {
                relative
                    .as_ref()
                    .and_then(|paths| paths.get(index))
                    .and_then(|path| normalize_path(path))
            })
            .or_else(|| from_sources.and_then(|path| normalize_path(path)))
            .or_else(fallback);
        paths.push(path);
    }
    paths
}

/// Normalizes a source path for debug metadata.
///
/// Forward slashes are canonical; a relative path is kept package-relative.
/// Absolute and drive-rooted paths are reduced to their final file name so a
/// host directory can never leak into the object.
fn normalize_path(path: &str) -> Option<String> {
    let mut path = path.replace('\\', "/");
    while path.starts_with("./") {
        path.drain(..2);
    }
    if path.is_empty() || path == "." {
        return None;
    }
    let absolute = path.starts_with('/') || path.as_bytes().get(1) == Some(&b':');
    if absolute {
        let name = path.rsplit('/').next().unwrap_or(&path);
        let name = name.rsplit_once(':').map_or(name, |(_, rest)| rest);
        if name.is_empty() {
            return None;
        }
        return Some(name.to_owned());
    }
    Some(path)
}

impl Lowerer<'_> {
    /// Returns the stable display name used for a source function, method, FB
    /// body, or program body.
    pub(crate) fn debug_function_name(&self, def: DefId) -> String {
        match &self.hir.defs[def.index()].kind {
            DefKind::Method { owner, .. } => {
                format!(
                    "{}.{}",
                    self.qualified_path(*owner),
                    self.qualified_path(def)
                )
            }
            _ => self.qualified_path(def),
        }
    }
}
