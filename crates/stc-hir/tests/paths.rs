//! Declaration paths and I/O binding-name candidates (D-253).

use stc_hir::{
    DefId, DefKind, HirCrate, PackageId, PackageInfo, RetainPolicy, TyId, VarSectionKind,
    Visibility,
};
use stc_span::{FileId, Span};

fn span(offset: u32) -> Span {
    Span::new(FileId::new(0), offset, offset + 1)
}

fn global(hir: &mut HirCrate, name: &str, parent: DefId) -> DefId {
    let symbol = hir.intern(name);
    let ty = TyId::new(0);
    hir.alloc_def(
        DefKind::Global {
            ty,
            section: VarSectionKind::Global,
            retain: RetainPolicy::None,
            constant: false,
            at: None,
            io: None,
            init: None,
        },
        Some(symbol),
        span(0),
        Some(parent),
        Visibility::Public,
    )
}

fn root_namespace(hir: &mut HirCrate, name: &str) -> DefId {
    let symbol = hir.intern(name);
    hir.alloc_def(
        DefKind::Namespace { internal: false },
        Some(symbol),
        span(0),
        None,
        Visibility::Public,
    )
}

#[test]
fn root_and_nested_declaration_paths() {
    let mut hir = HirCrate::new("sample", "Sample");
    let root = root_namespace(&mut hir, "Sample");
    let counter = global(&mut hir, "Counter", root);
    assert_eq!(hir.def_path(counter), Some(vec!["Counter".to_owned()]));

    let shared_symbol = hir.intern("Shared");
    let shared = hir.alloc_def(
        DefKind::Namespace { internal: false },
        Some(shared_symbol),
        span(1),
        Some(root),
        Visibility::Public,
    );
    let nested = global(&mut hir, "Counter", shared);
    assert_eq!(
        hir.def_path(nested),
        Some(vec!["Shared".to_owned(), "Counter".to_owned()])
    );
}

#[test]
fn declaration_named_like_the_package_namespace_keeps_its_name() {
    let mut hir = HirCrate::new("sample", "Sample");
    let root = root_namespace(&mut hir, "Sample");
    let same_name = global(&mut hir, "Sample", root);
    assert_eq!(
        hir.def_path(same_name),
        Some(vec!["Sample".to_owned()]),
        "the namespace-name pop must not remove a real declaration"
    );
}

#[test]
fn nested_members_keep_their_owner() {
    let mut hir = HirCrate::new("sample", "Sample");
    let root = root_namespace(&mut hir, "Sample");
    let block_symbol = hir.intern("Counter");
    let block = hir.alloc_def(
        DefKind::FunctionBlock {
            base: None,
            interfaces: Vec::new(),
            fields: Vec::new(),
            methods: Vec::new(),
            body: None,
            is_final: false,
            is_abstract: false,
            vtable: Vec::new(),
        },
        Some(block_symbol),
        span(2),
        Some(root),
        Visibility::Public,
    );
    let reset_symbol = hir.intern("Reset");
    let reset = hir.alloc_def(
        DefKind::Method {
            owner: block,
            result: None,
            params: Vec::new(),
            body: None,
            access: Visibility::Public,
            is_override: false,
            is_final: false,
            is_abstract: false,
            slot: None,
        },
        Some(reset_symbol),
        span(3),
        Some(block),
        Visibility::Public,
    );
    assert_eq!(
        hir.def_path(reset),
        Some(vec!["Counter".to_owned(), "Reset".to_owned()])
    );
}

#[test]
fn dependency_declaration_uses_its_own_package_prefixes() {
    let mut hir = HirCrate::with_packages(
        vec![
            PackageInfo {
                id: PackageId::new(0),
                name: "app".to_owned(),
                root_namespace: "App".to_owned(),
            },
            PackageInfo {
                id: PackageId::new(1),
                name: "lib".to_owned(),
                root_namespace: "Lib".to_owned(),
            },
        ],
        PackageId::new(0),
    );
    let root = root_namespace(&mut hir, "Lib");
    let counter = global(&mut hir, "Counter", root);
    hir.defs[counter.index()].package = PackageId::new(1);
    assert_eq!(
        hir.binding_name_candidates(counter),
        vec![
            "Counter".to_owned(),
            "Lib.Counter".to_owned(),
            "lib.Counter".to_owned()
        ]
    );
}

#[test]
fn binding_name_candidates_cover_short_qualified_and_package_spellings() {
    let mut hir = HirCrate::new("sample", "Sample");
    let root = root_namespace(&mut hir, "Sample");
    let shared_symbol = hir.intern("Shared");
    let shared = hir.alloc_def(
        DefKind::Namespace { internal: false },
        Some(shared_symbol),
        span(1),
        Some(root),
        Visibility::Public,
    );
    let counter = global(&mut hir, "Counter", shared);
    assert_eq!(
        hir.binding_name_candidates(counter),
        vec![
            "Counter".to_owned(),
            "Shared.Counter".to_owned(),
            "Sample.Shared.Counter".to_owned(),
            "sample.Shared.Counter".to_owned()
        ]
    );
}

#[test]
fn root_namespace_has_no_declaration_path() {
    let mut hir = HirCrate::new("sample", "Sample");
    let root = root_namespace(&mut hir, "Sample");
    assert_eq!(hir.def_path(root), None);
    assert!(hir.binding_name_candidates(root).is_empty());
}
