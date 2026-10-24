//! Stable dense identifiers used by HIR arenas.

macro_rules! define_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl $name {
            /// Creates an identifier from its arena index.
            #[must_use]
            pub const fn new(raw: u32) -> Self {
                Self(raw)
            }

            /// Returns the raw arena index.
            #[must_use]
            pub const fn raw(self) -> u32 {
                self.0
            }

            /// Returns the arena index as a `usize`.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

define_id!(
    /// Index of a definition in [`crate::HirCrate::defs`].
    DefId
);
define_id!(
    /// Index of a package in [`crate::HirCrate::packages`].
    PackageId
);
define_id!(
    /// Index of a type in [`crate::TyArena`].
    TyId
);
define_id!(
    /// Index of a field in [`crate::HirCrate::fields`].
    FieldId
);
define_id!(
    /// Index of a parameter in [`crate::HirCrate::params`].
    ParamId
);
define_id!(
    /// Index of a local variable in [`crate::HirCrate::locals`].
    LocalId
);
define_id!(
    /// Index of a body in [`crate::HirCrate::bodies`].
    BodyId
);
define_id!(
    /// Index of an expression in [`crate::HirCrate::exprs`].
    ExprId
);
define_id!(
    /// Index of a statement in [`crate::HirCrate::stmts`].
    StmtId
);
define_id!(
    /// Index of a standard function signature in [`crate::HirCrate::std_fns`].
    StdFnId
);
