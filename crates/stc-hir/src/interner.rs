//! Case-insensitive identifier interning (D-061).

use std::collections::HashMap;

/// A stable interned identifier.
///
/// Equality is case-insensitive because [`Interner::intern`] keys on the
/// uppercase ASCII spelling. The first spelling seen is preserved for
/// diagnostics; name nodes carry their own spelling for reference sites.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Symbol(u32);

impl Symbol {
    /// Creates a symbol from its raw index. Intended for tests and dumps.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Returns the raw index.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// Returns the index as a `usize`.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Debug)]
struct Entry {
    text: Box<str>,
    normalized: Box<str>,
}

/// Interns identifiers by their uppercase ASCII key.
#[derive(Debug, Default)]
pub struct Interner {
    entries: Vec<Entry>,
    by_key: HashMap<String, Symbol>,
}

impl Interner {
    /// Creates an empty interner.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns `text` and returns its symbol.
    #[must_use]
    pub fn intern(&mut self, text: &str) -> Symbol {
        let normalized = text.to_ascii_uppercase();
        if let Some(symbol) = self.by_key.get(&normalized) {
            return *symbol;
        }
        let raw = u32::try_from(self.entries.len()).expect("symbol limit exceeded");
        let symbol = Symbol(raw);
        self.entries.push(Entry {
            text: text.into(),
            normalized: normalized.clone().into_boxed_str(),
        });
        self.by_key.insert(normalized, symbol);
        symbol
    }

    /// Returns the symbol for `text` when it has already been interned.
    #[must_use]
    pub fn get(&self, text: &str) -> Option<Symbol> {
        self.by_key.get(&text.to_ascii_uppercase()).copied()
    }

    /// Returns the first original spelling of `symbol`.
    #[must_use]
    pub fn text(&self, symbol: Symbol) -> &str {
        self.entries
            .get(symbol.index())
            .map_or("", |entry| &entry.text)
    }

    /// Returns the uppercase lookup key of `symbol`.
    #[must_use]
    pub fn normalized(&self, symbol: Symbol) -> &str {
        self.entries
            .get(symbol.index())
            .map_or("", |entry| &entry.normalized)
    }

    /// Returns the number of interned symbols.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when nothing has been interned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::Interner;

    #[test]
    fn interning_is_case_insensitive_and_keeps_the_first_spelling() {
        let mut interner = Interner::new();
        let first = interner.intern("Motor");
        let second = interner.intern("MOTOR");
        assert_eq!(first, second);
        assert_eq!(interner.text(first), "Motor");
        assert_eq!(interner.normalized(first), "MOTOR");
        assert_eq!(interner.get("mOtOr"), Some(first));
    }
}
