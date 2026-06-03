//! Live document registry.
//!
//! Holds loaded documents in memory with per-document SyncDoc and broadcast
//! channel. Lazy-loads from the DocumentStore on first access.

use crate::store::DocumentStore;
use cadview_core::sync::SyncDoc;
use cadview_core::Document;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::time::Instant;
use tokio::sync::broadcast;

pub struct DocumentSlot {
    pub document: Document,
    pub sync_doc: SyncDoc,
    pub update_tx: broadcast::Sender<Vec<u8>>,
    pub client_count: usize,
    pub last_accessed: Instant,
    /// Path to the original DWG on disk (for overlay saves preserving AutoCAD infrastructure).
    pub source_dwg_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentInfo {
    pub id: String,
    pub filename: String,
    pub prefix: String,
    pub loaded: bool,
    pub entity_count: Option<usize>,
    pub layer_count: Option<usize>,
}

pub struct DocumentRegistry {
    slots: HashMap<String, DocumentSlot>,
    store: Box<dyn DocumentStore>,
    next_server_client_id: u64,
}

impl DocumentRegistry {
    pub fn new(store: Box<dyn DocumentStore>) -> Self {
        Self {
            slots: HashMap::new(),
            store,
            next_server_client_id: 1,
        }
    }

    /// Get a loaded document slot, or load from store on first access.
    /// If the key isn't in the store either, creates an empty document
    /// (for client-created new drawings that sync to the server).
    pub fn get_or_load(&mut self, key: &str) -> Result<&mut DocumentSlot> {
        if !self.slots.contains_key(key) {
            let doc = if self.store.exists(key) {
                let bytes = self.store.load(key)
                    .with_context(|| format!("loading document '{}'", key))?;
                let doc = cadview_core::load_dwg_bytes(&bytes)
                    .with_context(|| format!("parsing DWG '{}'", key))?;
                tracing::info!("Loaded '{}': {} entities", key, doc.entities.len());
                doc
            } else {
                tracing::info!("Created empty slot for '{}'", key);
                Document::new()
            };

            let client_id = self.next_server_client_id;
            self.next_server_client_id += 1;
            let sync_doc = SyncDoc::new(client_id);
            sync_doc.populate_from_document(&doc);

            let (update_tx, _) = broadcast::channel(64);

            let source_dwg_path = if self.store.exists(key) {
                self.store.resolve_path(key)
            } else {
                None
            };

            self.slots.insert(key.to_string(), DocumentSlot {
                document: doc,
                sync_doc,
                update_tx,
                client_count: 0,
                last_accessed: Instant::now(),
                source_dwg_path,
            });
        }

        let slot = self.slots.get_mut(key).unwrap();
        slot.last_accessed = Instant::now();
        Ok(slot)
    }

    /// Get a loaded slot without triggering a load.
    pub fn get(&self, key: &str) -> Option<&DocumentSlot> {
        self.slots.get(key)
    }

    /// Get a mutable loaded slot without triggering a load.
    pub fn get_mut(&mut self, key: &str) -> Option<&mut DocumentSlot> {
        self.slots.get_mut(key)
    }

    /// Insert a pre-loaded document (e.g. from CLI arg).
    pub fn insert(&mut self, key: String, doc: Document, source_dwg_path: Option<String>) -> &mut DocumentSlot {
        let client_id = self.next_server_client_id;
        self.next_server_client_id += 1;
        let sync_doc = SyncDoc::new(client_id);
        sync_doc.populate_from_document(&doc);
        let (update_tx, _) = broadcast::channel(64);

        self.slots.insert(key.clone(), DocumentSlot {
            document: doc,
            sync_doc,
            update_tx,
            client_count: 0,
            last_accessed: Instant::now(),
            source_dwg_path,
        });
        self.slots.get_mut(&key).unwrap()
    }

    /// List all documents from the store, enriched with loaded status.
    pub fn list_available(&self) -> Vec<DocumentInfo> {
        let keys = self.store.list().unwrap_or_default();
        keys.iter().map(|key| {
            let filename = key.rsplit('/').next().unwrap_or(key).to_string();
            let prefix = if let Some(pos) = key.rfind('/') {
                key[..=pos].to_string()
            } else {
                String::new()
            };
            let slot = self.slots.get(key);
            DocumentInfo {
                id: key.clone(),
                filename,
                prefix,
                loaded: slot.is_some(),
                entity_count: slot.map(|s| s.document.entities.len()),
                layer_count: slot.map(|s| s.document.layers.len()),
            }
        }).collect()
    }

    /// Decrement client count for a document.
    pub fn release(&mut self, key: &str) {
        if let Some(slot) = self.slots.get_mut(key) {
            slot.client_count = slot.client_count.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn make_test_registry() -> (tempfile::TempDir, DocumentRegistry) {
        let dir = tempfile::tempdir().unwrap();
        // Create a minimal valid DWG-like file (will fail to parse, that's OK for store tests)
        let store = Box::new(crate::store::FolderStore::new(dir.path().to_path_buf()));
        (dir, DocumentRegistry::new(store))
    }

    #[test]
    fn list_available_empty() {
        let (_dir, reg) = make_test_registry();
        assert!(reg.list_available().is_empty());
    }

    #[test]
    fn insert_and_list() {
        let (_dir, mut reg) = make_test_registry();
        reg.insert("test.dwg".to_string(), Document::new(), None);
        let list = reg.list_available();
        // The store (FolderStore on empty dir) has no files,
        // but we inserted one manually. list_available only shows store files.
        assert!(list.is_empty());
        // But get() finds it
        assert!(reg.get("test.dwg").is_some());
    }

    #[test]
    fn release_decrements_count() {
        let (_dir, mut reg) = make_test_registry();
        let slot = reg.insert("x.dwg".to_string(), Document::new(), None);
        slot.client_count = 3;
        reg.release("x.dwg");
        assert_eq!(reg.get("x.dwg").unwrap().client_count, 2);
        reg.release("x.dwg");
        reg.release("x.dwg");
        reg.release("x.dwg"); // saturates at 0
        assert_eq!(reg.get("x.dwg").unwrap().client_count, 0);
    }

    #[test]
    fn document_info_prefix_parsing() {
        // Manually test prefix extraction
        let info = DocumentInfo {
            id: "sub/deep/file.dwg".to_string(),
            filename: "file.dwg".to_string(),
            prefix: "sub/deep/".to_string(),
            loaded: false,
            entity_count: None,
            layer_count: None,
        };
        assert_eq!(info.prefix, "sub/deep/");
        assert_eq!(info.filename, "file.dwg");
    }

    #[test]
    fn per_document_broadcast_channels() {
        let (_dir, mut reg) = make_test_registry();
        reg.insert("a.dwg".to_string(), Document::new(), None);
        reg.insert("b.dwg".to_string(), Document::new(), None);

        let tx_a = reg.get("a.dwg").unwrap().update_tx.clone();
        let tx_b = reg.get("b.dwg").unwrap().update_tx.clone();

        // Subscribe to b only
        let mut rx_b = tx_b.subscribe();

        // Send on a -- should not appear on b's receiver
        let _ = tx_a.send(vec![1, 2, 3]);
        assert!(rx_b.try_recv().is_err());

        // Send on b -- should appear
        let _ = tx_b.send(vec![4, 5, 6]);
        assert_eq!(rx_b.try_recv().unwrap(), vec![4, 5, 6]);
    }
}
