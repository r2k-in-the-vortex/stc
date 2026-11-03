//! The open-document store.
//!
//! Text documents are keyed by their normalized URI. The store holds the
//! client's unsaved text; the project snapshot receives the same text as an
//! overlay. Documents outside the project are still tracked so that a later
//! project reload can apply them, but queries answer empty results for them.

use std::collections::BTreeMap;
use std::path::PathBuf;

use url::Url;

use crate::uri::{normalize_uri, uri_to_path};

/// One open text document.
#[derive(Clone, Debug)]
pub struct Document {
    /// The document's file URI.
    pub uri: Url,
    /// The path the URI names.
    pub path: PathBuf,
    /// The client's current text (unsaved).
    pub text: String,
    /// The client's document version.
    pub version: i64,
}

/// Open documents keyed by normalized URI.
#[derive(Debug, Default)]
pub struct DocumentStore {
    documents: BTreeMap<String, Document>,
}

impl DocumentStore {
    /// Creates an empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an opened document.
    ///
    /// Returns `None` when `uri` is not a file URI; nothing is stored then.
    pub fn open(&mut self, uri: &str, text: String, version: i64) -> Option<&Document> {
        let parsed = Url::parse(uri).ok()?;
        let path = uri_to_path(uri)?;
        let key = normalize_uri(uri);
        self.documents.insert(
            key.clone(),
            Document {
                uri: parsed,
                path,
                text,
                version,
            },
        );
        self.documents.get(&key)
    }

    /// Replaces a document's text for a full-sync change.
    pub fn change(&mut self, uri: &str, text: String, version: i64) -> Option<&Document> {
        let key = normalize_uri(uri);
        let document = self.documents.get_mut(&key)?;
        document.text = text;
        document.version = version;
        Some(document)
    }

    /// Records a save, optionally replacing the text.
    pub fn save(&mut self, uri: &str, text: Option<String>) -> Option<&Document> {
        let key = normalize_uri(uri);
        let document = self.documents.get_mut(&key)?;
        if let Some(text) = text {
            document.text = text;
        }
        Some(document)
    }

    /// Removes a closed document.
    pub fn close(&mut self, uri: &str) -> Option<Document> {
        self.documents.remove(&normalize_uri(uri))
    }

    /// Returns an open document.
    #[must_use]
    pub fn get(&self, uri: &str) -> Option<&Document> {
        self.documents.get(&normalize_uri(uri))
    }

    /// Iterates over open documents.
    pub fn documents(&self) -> impl Iterator<Item = &Document> {
        self.documents.values()
    }

    /// Returns the number of open documents.
    #[must_use]
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    /// Returns `true` when no document is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uri::path_to_uri;

    fn uri_for(name: &str) -> String {
        path_to_uri(&std::env::temp_dir().join(name))
            .expect("temp file URI")
            .to_string()
    }

    #[test]
    fn tracks_open_change_save_and_close() {
        let uri = uri_for("stc docs ünïcode.st");
        let mut store = DocumentStore::new();
        let document = store
            .open(&uri, "first".to_owned(), 1)
            .expect("file document");
        assert_eq!(document.text, "first");
        assert_eq!(store.len(), 1);
        assert_eq!(store.get(&uri).expect("open").version, 1);

        store.change(&uri, "second".to_owned(), 2).expect("change");
        assert_eq!(store.get(&uri).expect("open").text, "second");

        store.save(&uri, Some("third".to_owned())).expect("save");
        assert_eq!(store.get(&uri).expect("open").text, "third");
        store.save(&uri, None).expect("save without text");
        assert_eq!(store.get(&uri).expect("open").version, 2);

        let closed = store.close(&uri).expect("closed document");
        assert_eq!(
            closed.path,
            std::env::temp_dir().join("stc docs ünïcode.st")
        );
        assert!(store.is_empty());
        assert!(store.get(&uri).is_none());
    }

    #[test]
    fn rejects_non_file_uris() {
        let mut store = DocumentStore::new();
        assert!(store.open("untitled:Untitled-1", "x".into(), 1).is_none());
        assert!(store.is_empty());
        assert!(store.change("untitled:Untitled-1", "y".into(), 2).is_none());
    }
}
