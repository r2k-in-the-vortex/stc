//! Canonical HIR types (AST §6, D-063).

use std::collections::BTreeMap;

use crate::DefId;
use crate::id::TyId;

/// A signed or unsigned integer width in bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IntTy {
    /// `true` for `SINT`/`INT`/`DINT`/`LINT`.
    pub signed: bool,
    /// Width in bits: 8, 16, 32, or 64.
    pub width: u8,
}

impl IntTy {
    /// Creates an integer type.
    #[must_use]
    pub const fn new(signed: bool, width: u8) -> Self {
        Self { signed, width }
    }
}

/// A floating-point width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RealTy {
    /// `REAL` (binary32).
    Real,
    /// `LREAL` (binary64).
    LReal,
}

/// A duration type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeTy {
    /// `TIME`.
    Time,
    /// `LTIME`.
    LTime,
}

/// A date/time category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DateTy {
    /// `DATE`.
    Date,
    /// `LDATE`.
    LDate,
    /// `TIME_OF_DAY`/`TOD`.
    Tod,
    /// `LTOD`/`LTIME_OF_DAY`.
    LTod,
    /// `DATE_AND_TIME`/`DT`.
    DateTime,
    /// `LDT`/`LDATE_AND_TIME`.
    LDateTime,
}

/// A character type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CharTy {
    /// `CHAR`.
    Char,
    /// `WCHAR`.
    WChar,
    /// `UCHAR` (32-bit Unicode scalar, `STC-UTF8`).
    UChar,
}

/// String encoding category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StringKind {
    /// `STRING` (single-byte units).
    Sbcs,
    /// `WSTRING` (UTF-16 code units).
    Dbcs,
    /// `USTRING` (Unicode scalars, 4 bytes each).
    Unicode,
}

/// A fixed-capacity string type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StringTy {
    /// Encoding category.
    pub kind: StringKind,
    /// Capacity in characters/units.
    pub capacity: u32,
}

/// One array dimension bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArrayBoundTy {
    /// Inclusive lower bound.
    pub lower: i128,
    /// Inclusive upper bound.
    pub upper: i128,
}

/// A static or conformant array type.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArrayTy {
    /// Element type.
    pub element: TyId,
    /// Bounds, outermost first.
    pub bounds: Vec<ArrayBoundTy>,
    /// `true` for `ARRAY [*]` conformant parameters.
    pub conformant: bool,
}

impl ArrayTy {
    /// Returns the number of elements in a static array, or `None` for a
    /// conformant array or an invalid bound set.
    #[must_use]
    pub fn element_count(&self) -> Option<i128> {
        if self.conformant {
            return None;
        }
        let mut total: i128 = 1;
        for bound in &self.bounds {
            let length = bound.upper.checked_sub(bound.lower)?.checked_add(1)?;
            if length <= 0 {
                return None;
            }
            total = total.checked_mul(length)?;
        }
        Some(total)
    }
}

/// A reference type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RefTy {
    /// Referenced type.
    pub target: TyId,
}

/// A transient raw pointer type (`STC-POINTER`, D-145).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PtrTy {
    /// Pointee type.
    pub target: TyId,
}

/// Standard-function generic category (`ANY_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GenericKind {
    /// `ANY`.
    Any,
    /// `ANY_DERIVED`.
    AnyDerived,
    /// `ANY_ELEMENTARY`.
    AnyElementary,
    /// `ANY_MAGNITUDE`.
    AnyMagnitude,
    /// `ANY_NUM`.
    AnyNum,
    /// `ANY_REAL`.
    AnyReal,
    /// `ANY_INT`.
    AnyInt,
    /// `ANY_UNSIGNED`.
    AnyUnsigned,
    /// `ANY_SIGNED`.
    AnySigned,
    /// `ANY_DURATION`.
    AnyDuration,
    /// `ANY_BIT`.
    AnyBit,
    /// `ANY_CHARS`.
    AnyChars,
    /// `ANY_STRING`.
    AnyString,
    /// `ANY_CHAR`.
    AnyChar,
    /// `ANY_DATE`.
    AnyDate,
}

impl GenericKind {
    /// Returns the canonical `ANY_*` spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Any => "ANY",
            Self::AnyDerived => "ANY_DERIVED",
            Self::AnyElementary => "ANY_ELEMENTARY",
            Self::AnyMagnitude => "ANY_MAGNITUDE",
            Self::AnyNum => "ANY_NUM",
            Self::AnyReal => "ANY_REAL",
            Self::AnyInt => "ANY_INT",
            Self::AnyUnsigned => "ANY_UNSIGNED",
            Self::AnySigned => "ANY_SIGNED",
            Self::AnyDuration => "ANY_DURATION",
            Self::AnyBit => "ANY_BIT",
            Self::AnyChars => "ANY_CHARS",
            Self::AnyString => "ANY_STRING",
            Self::AnyChar => "ANY_CHAR",
            Self::AnyDate => "ANY_DATE",
        }
    }
}

/// A canonical HIR type.
///
/// `Named` refers to a nominal type declaration; its structure lives in
/// [`crate::DefKind`]. Aliases are resolved away before a `Ty` is built.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Ty {
    /// The cascade suppressor.
    Error,
    /// `BOOL`.
    Bool,
    /// Signed or unsigned integer.
    Int(IntTy),
    /// `REAL` or `LREAL`.
    Real(RealTy),
    /// `BYTE`/`WORD`/`DWORD`/`LWORD`, width in bits.
    BitString(u8),
    /// `TIME` or `LTIME`.
    Time(TimeTy),
    /// Date/time category.
    Date(DateTy),
    /// `CHAR`/`WCHAR`/`UCHAR`.
    Char(CharTy),
    /// Fixed-capacity string.
    String(StringTy),
    /// Static or conformant array.
    Array(ArrayTy),
    /// Reference.
    Ref(RefTy),
    /// Transient raw pointer (`STC-POINTER`).
    Ptr(PtrTy),
    /// Nominal user type (enum, subrange, struct, class, FB, interface, ...).
    Named(DefId),
    /// The type of `NULL` (internal compatibility type).
    Null,
    /// Standard-function generic category.
    Generic(GenericKind),
}

/// A canonicalizing type arena.
#[derive(Debug)]
pub struct TyArena {
    types: Vec<Ty>,
    by_key: BTreeMap<Ty, TyId>,
}

impl TyArena {
    /// Creates an arena containing all built-in scalar types.
    #[must_use]
    pub fn new() -> Self {
        let mut arena = Self {
            types: Vec::new(),
            by_key: BTreeMap::new(),
        };
        for ty in builtins() {
            let _ = arena.intern(ty);
        }
        arena
    }

    /// Interns a type and returns its canonical ID.
    #[must_use]
    pub fn intern(&mut self, ty: Ty) -> TyId {
        if let Some(id) = self.by_key.get(&ty) {
            return *id;
        }
        let raw = u32::try_from(self.types.len()).expect("type limit exceeded");
        let id = TyId::new(raw);
        self.types.push(ty.clone());
        self.by_key.insert(ty, id);
        id
    }

    /// Returns the type for an ID.
    ///
    /// # Panics
    ///
    /// Panics if `id` is not in this arena.
    #[must_use]
    pub fn get(&self, id: TyId) -> &Ty {
        &self.types[id.index()]
    }

    /// Returns all types in ID order.
    pub fn iter(&self) -> impl Iterator<Item = (TyId, &Ty)> {
        self.types
            .iter()
            .enumerate()
            .map(|(index, ty)| (TyId::new(index as u32), ty))
    }

    /// Returns the number of interned types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.types.len()
    }

    /// Returns `true` when the arena is empty (never for a constructed arena).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    /// Returns the shared error type.
    #[must_use]
    pub fn error(&self) -> TyId {
        self.id_of(&Ty::Error)
    }

    /// Returns the shared `BOOL` type.
    #[must_use]
    pub fn bool(&self) -> TyId {
        self.id_of(&Ty::Bool)
    }

    /// Returns the shared `NULL` type.
    #[must_use]
    pub fn null(&self) -> TyId {
        self.id_of(&Ty::Null)
    }

    /// Returns a pointer type ID, interning a new pointee on demand.
    #[must_use]
    pub fn pointer(&mut self, target: TyId) -> TyId {
        self.intern(Ty::Ptr(PtrTy { target }))
    }

    /// Returns an integer type ID.
    #[must_use]
    pub fn int(&self, signed: bool, width: u8) -> TyId {
        self.id_of(&Ty::Int(IntTy::new(signed, width)))
    }

    /// Returns a real type ID.
    #[must_use]
    pub fn real(&self, ty: RealTy) -> TyId {
        self.id_of(&Ty::Real(ty))
    }

    /// Returns a bit-string type ID.
    #[must_use]
    pub fn bit_string(&self, width: u8) -> TyId {
        self.id_of(&Ty::BitString(width))
    }

    /// Returns a time type ID.
    #[must_use]
    pub fn time(&self, ty: TimeTy) -> TyId {
        self.id_of(&Ty::Time(ty))
    }

    /// Returns a date/time type ID.
    #[must_use]
    pub fn date(&self, ty: DateTy) -> TyId {
        self.id_of(&Ty::Date(ty))
    }

    /// Returns a character type ID.
    #[must_use]
    pub fn char_ty(&self, ty: CharTy) -> TyId {
        self.id_of(&Ty::Char(ty))
    }

    /// Returns a string type ID, interning new capacities on demand.
    #[must_use]
    pub fn string(&mut self, kind: StringKind, capacity: u32) -> TyId {
        self.intern(Ty::String(StringTy { kind, capacity }))
    }

    fn id_of(&self, ty: &Ty) -> TyId {
        *self
            .by_key
            .get(ty)
            .unwrap_or_else(|| panic!("builtin type {ty:?} missing from the arena"))
    }
}

impl Default for TyArena {
    fn default() -> Self {
        Self::new()
    }
}

fn builtins() -> Vec<Ty> {
    let mut types = vec![Ty::Error, Ty::Bool, Ty::Null];
    for width in [8, 16, 32, 64] {
        types.push(Ty::Int(IntTy::new(true, width)));
        types.push(Ty::Int(IntTy::new(false, width)));
        types.push(Ty::BitString(width));
    }
    types.push(Ty::Real(RealTy::Real));
    types.push(Ty::Real(RealTy::LReal));
    types.push(Ty::Time(TimeTy::Time));
    types.push(Ty::Time(TimeTy::LTime));
    for date in [
        DateTy::Date,
        DateTy::LDate,
        DateTy::Tod,
        DateTy::LTod,
        DateTy::DateTime,
        DateTy::LDateTime,
    ] {
        types.push(Ty::Date(date));
    }
    for ch in [CharTy::Char, CharTy::WChar, CharTy::UChar] {
        types.push(Ty::Char(ch));
    }
    for kind in [StringKind::Sbcs, StringKind::Dbcs, StringKind::Unicode] {
        types.push(Ty::String(StringTy {
            kind,
            capacity: 255,
        }));
    }
    types
}
