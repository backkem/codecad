//! Yrs CRDT sync for cadview documents.
//!
//! Single implementation used by both browser (WASM) and server (native).
//! Both are equal-weight peers: same code path for mutations, diffing,
//! and update encoding. The server adds forwarding and persistence on top.
//!
//! Entities and layers are stored as bincode blobs in Yrs YMaps.
//! Lossless roundtrip, no intermediate format.

use crate::{Document, EntityId};
use std::collections::HashMap;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Any, Doc, Map, Out, ReadTxn, Transact};

/// Shared Yrs-backed document state. Each peer (browser or server) owns one.
///
/// Root maps are created at construction per yrs docs: "define all root
/// shared types during document creation" to avoid transaction deadlocks.
pub struct SyncDoc {
    pub doc: Doc,
    entities: yrs::MapRef,
    layers: yrs::MapRef,
    blocks: yrs::MapRef,
    linetypes: yrs::MapRef,
    meta: yrs::MapRef,
    /// When Some, we're inside a batch. Stores the entity hashes and layer
    /// names captured at begin_batch. Diffs are deferred until end_batch.
    batch_state: std::cell::RefCell<Option<BatchState>>,
}

struct BatchState {
    ent_before: HashMap<u64, u64>,
    layer_before: std::collections::HashSet<String>,
    block_before: std::collections::HashSet<String>,
    linetype_before: std::collections::HashSet<String>,
}

impl SyncDoc {
    pub fn new(client_id: u64) -> Self {
        let doc = Doc::with_client_id(client_id);
        let entities = doc.get_or_insert_map("entities");
        let layers = doc.get_or_insert_map("layers");
        let blocks = doc.get_or_insert_map("blocks");
        let linetypes = doc.get_or_insert_map("linetypes");
        let meta = doc.get_or_insert_map("meta");
        Self {
            doc,
            entities,
            layers,
            blocks,
            linetypes,
            meta,
            batch_state: std::cell::RefCell::new(None),
        }
    }

    // ── Bulk operations ────────────────────────────────────────────────

    /// Populate the Yrs Doc from a Document (e.g. after loading a DWG).
    /// No-op on empty documents (avoids generating degenerate Yrs updates
    /// from clearing already-empty maps).
    pub fn populate_from_document(&self, doc: &Document) {
        if doc.entities.is_empty() && doc.layers.is_empty() {
            return;
        }

        let mut txn = self.doc.transact_mut();

        self.entities.clear(&mut txn);
        self.layers.clear(&mut txn);
        self.blocks.clear(&mut txn);
        self.linetypes.clear(&mut txn);

        for ent in &doc.entities {
            let key = format!("e_{}", ent.id.0);
            let bytes = crate::entity_to_bytes(ent);
            self.entities.insert(&mut txn, key.as_str(), bytes);
        }

        for layer in &doc.layers {
            let bytes = crate::layer_to_bytes(layer);
            self.layers.insert(&mut txn, layer.name.as_str(), bytes);
        }

        for (name, block) in &doc.blocks {
            let bytes = crate::block_to_bytes(block);
            self.blocks.insert(&mut txn, name.as_str(), bytes);
        }

        for (name, lt) in &doc.linetypes {
            let bytes = crate::linetype_to_bytes(lt);
            self.linetypes.insert(&mut txn, name.as_str(), bytes);
        }

        let max_id = doc.entities.iter().map(|e| e.id.0).max().unwrap_or(0);
        self.meta.insert(&mut txn, "next_id", (max_id + 1) as f64);
    }

    /// Rebuild a Document from the current Yrs state.
    pub fn to_document(&self) -> Document {
        let txn = self.doc.transact();
        let mut doc = Document::new();

        for (_key, value) in self.layers.iter(&txn) {
            if let Out::Any(Any::Buffer(buf)) = value {
                if let Some(layer) = crate::layer_from_bytes(&buf) {
                    doc.layers.push(layer);
                }
            }
        }

        for (_key, value) in self.entities.iter(&txn) {
            if let Out::Any(Any::Buffer(buf)) = value {
                if let Some(ent) = crate::entity_from_bytes(&buf) {
                    doc.entities.push(ent);
                }
            }
        }

        for (key, value) in self.blocks.iter(&txn) {
            if let Out::Any(Any::Buffer(buf)) = value {
                if let Some(block) = crate::block_from_bytes(&buf) {
                    doc.blocks.insert(key.to_string(), block);
                }
            }
        }

        for (key, value) in self.linetypes.iter(&txn) {
            if let Out::Any(Any::Buffer(buf)) = value {
                if let Some(lt) = crate::linetype_from_bytes(&buf) {
                    doc.linetypes.insert(key.to_string(), lt);
                }
            }
        }

        if let Some(Out::Any(Any::Number(n))) = self.meta.get(&txn, "next_id") {
            doc.set_next_id(n as u64);
        }

        // Preserve draw order
        doc.entities.sort_by_key(|e| e.id.0);
        doc
    }

    // ── Mutation with automatic Yrs diff ───────────────────────────────

    /// Run a cad_call mutation on the Document, then diff the result into
    /// the Yrs Doc. Returns (cad_call result, incremental Yrs update bytes).
    ///
    /// The update bytes are what you send to peers. Empty if no changes.
    pub fn apply_mutation(
        &self,
        local_doc: &mut Document,
        method: &str,
        args: &str,
    ) -> Result<(String, Vec<u8>), String> {
        // Inside a batch: run cad_call, skip diff (end_batch diffs once)
        if self.batch_state.borrow().is_some() {
            let result = crate::cad_call(local_doc, method, args)?;
            return Ok((result, Vec::new()));
        }

        let ent_before = entity_hashes(local_doc);
        let layer_before = layer_names(local_doc);
        let block_before = block_names(local_doc);
        let lt_before = linetype_names(local_doc);

        let result = crate::cad_call(local_doc, method, args)?;

        let ent_after = entity_hashes(local_doc);
        let layer_after = layer_names(local_doc);
        let block_after = block_names(local_doc);
        let lt_after = linetype_names(local_doc);

        let update = if ent_before != ent_after
            || layer_before != layer_after
            || block_before != block_after
            || lt_before != lt_after
        {
            self.diff_into_yrs(
                local_doc,
                &ent_before,
                &ent_after,
                &layer_before,
                &layer_after,
                &block_before,
                &block_after,
                &lt_before,
                &lt_after,
            )
        } else {
            Vec::new()
        };

        Ok((result, update))
    }

    /// Start a batch. Mutations via apply_mutation will skip per-call diffs.
    /// Call end_batch to produce one combined Yrs update.
    pub fn begin_batch(&self, local_doc: &Document) {
        *self.batch_state.borrow_mut() = Some(BatchState {
            ent_before: entity_hashes(local_doc),
            layer_before: layer_names(local_doc),
            block_before: block_names(local_doc),
            linetype_before: linetype_names(local_doc),
        });
    }

    /// End a batch and produce a single Yrs update covering all mutations
    /// since begin_batch. Returns empty vec if nothing changed.
    pub fn end_batch(&self, local_doc: &Document) -> Vec<u8> {
        let state = self.batch_state.borrow_mut().take();
        let Some(bs) = state else { return Vec::new() };

        let ent_after = entity_hashes(local_doc);
        let layer_after = layer_names(local_doc);
        let block_after = block_names(local_doc);
        let lt_after = linetype_names(local_doc);

        if bs.ent_before != ent_after
            || bs.layer_before != layer_after
            || bs.block_before != block_after
            || bs.linetype_before != lt_after
        {
            self.diff_into_yrs(
                local_doc,
                &bs.ent_before,
                &ent_after,
                &bs.layer_before,
                &layer_after,
                &bs.block_before,
                &block_after,
                &bs.linetype_before,
                &lt_after,
            )
        } else {
            Vec::new()
        }
    }

    /// Diff Document changes into the Yrs Doc and return the incremental
    /// update bytes. Generic over any before/after entity state.
    #[allow(clippy::too_many_arguments)]
    fn diff_into_yrs(
        &self,
        doc: &Document,
        ent_before: &HashMap<u64, u64>,
        ent_after: &HashMap<u64, u64>,
        layer_before: &std::collections::HashSet<String>,
        layer_after: &std::collections::HashSet<String>,
        block_before: &std::collections::HashSet<String>,
        block_after: &std::collections::HashSet<String>,
        lt_before: &std::collections::HashSet<String>,
        lt_after: &std::collections::HashSet<String>,
    ) -> Vec<u8> {
        let sv_before = {
            let txn = self.doc.transact();
            txn.state_vector()
        };

        let mut txn = self.doc.transact_mut();

        // Removed entities
        for id in ent_before.keys() {
            if !ent_after.contains_key(id) {
                self.entities.remove(&mut txn, &format!("e_{}", id));
            }
        }

        // Added or changed entities
        for (id, hash) in ent_after {
            if ent_before.get(id) != Some(hash) {
                if let Some(ent) = doc.entity(EntityId(*id)) {
                    let key = format!("e_{}", id);
                    let bytes = crate::entity_to_bytes(ent);
                    self.entities.insert(&mut txn, key.as_str(), bytes);
                }
            }
        }

        // Update next_id
        if let Some(max_id) = ent_after.keys().max() {
            self.meta.insert(&mut txn, "next_id", (*max_id + 1) as f64);
        }

        // Removed layers
        for name in layer_before {
            if !layer_after.contains(name) {
                self.layers.remove(&mut txn, name.as_str());
            }
        }

        // Added or changed layers
        for layer in &doc.layers {
            let bytes = crate::layer_to_bytes(layer);
            self.layers.insert(&mut txn, layer.name.as_str(), bytes);
        }

        // Removed blocks
        for name in block_before {
            if !block_after.contains(name) {
                self.blocks.remove(&mut txn, name.as_str());
            }
        }

        // Added or changed blocks
        for name in block_after {
            if !block_before.contains(name) {
                if let Some(block) = doc.blocks.get(name) {
                    let bytes = crate::block_to_bytes(block);
                    self.blocks.insert(&mut txn, name.as_str(), bytes);
                }
            }
        }

        // Removed linetypes
        for name in lt_before {
            if !lt_after.contains(name) {
                self.linetypes.remove(&mut txn, name.as_str());
            }
        }

        // Added or changed linetypes
        for name in lt_after {
            if !lt_before.contains(name) {
                if let Some(lt) = doc.linetypes.get(name) {
                    let bytes = crate::linetype_to_bytes(lt);
                    self.linetypes.insert(&mut txn, name.as_str(), bytes);
                }
            }
        }

        drop(txn);

        // Encode incremental update
        let txn = self.doc.transact();
        txn.encode_state_as_update_v1(&sv_before)
    }

    // ── Wire protocol helpers ──────────────────────────────────────────
    // These are the building blocks for the sync handshake. Both peers
    // call the same functions.

    /// Encode our state vector (for sync handshake step 1).
    pub fn state_vector(&self) -> Vec<u8> {
        let txn = self.doc.transact();
        txn.state_vector().encode_v1()
    }

    /// Encode full state as an update (for recovery / initial sync).
    pub fn encode_state(&self) -> Vec<u8> {
        let txn = self.doc.transact();
        txn.encode_state_as_update_v1(&yrs::StateVector::default())
    }

    /// Encode the diff between our state and a remote state vector.
    pub fn encode_diff(&self, remote_sv: &[u8]) -> anyhow::Result<Vec<u8>> {
        let sv = yrs::StateVector::decode_v1(remote_sv)?;
        let txn = self.doc.transact();
        Ok(txn.encode_state_as_update_v1(&sv))
    }

    /// Apply an incoming Yrs update from a peer.
    /// Trivial updates (empty or [0,0]) are no-ops.
    pub fn apply_update(&self, update: &[u8]) -> anyhow::Result<()> {
        if update.is_empty() || update == [0, 0] {
            return Ok(());
        }
        let mut txn = self.doc.transact_mut();
        txn.apply_update(yrs::Update::decode_v1(update)?)?;
        Ok(())
    }
}

fn entity_hashes(doc: &Document) -> HashMap<u64, u64> {
    use std::hash::{Hash, Hasher};
    doc.entities
        .iter()
        .map(|e| {
            let bytes = crate::entity_to_bytes(e);
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            bytes.hash(&mut hasher);
            (e.id.0, hasher.finish())
        })
        .collect()
}

fn layer_names(doc: &Document) -> std::collections::HashSet<String> {
    doc.layers.iter().map(|l| l.name.clone()).collect()
}

fn block_names(doc: &Document) -> std::collections::HashSet<String> {
    doc.blocks.keys().cloned().collect()
}

fn linetype_names(doc: &Document) -> std::collections::HashSet<String> {
    doc.linetypes.keys().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;
    use kurbo::Point;

    #[test]
    fn populate_and_rebuild_roundtrip() {
        let mut doc = Document::new();
        doc.add_layer("WALLS", Color::rgb(255, 0, 0));
        doc.add_line(
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            "WALLS",
            Some(Color::rgb(255, 0, 0)),
        );
        doc.add_circle(
            Point::new(50.0, 50.0),
            25.0,
            "ELEC",
            Some(Color::rgb(0, 255, 0)),
        );
        doc.add_polyline(
            vec![
                Point::new(0.0, 0.0),
                Point::new(10.0, 0.0),
                Point::new(10.0, 10.0),
            ],
            true,
            "WALLS",
            Some(Color::rgb(255, 0, 0)),
        );

        let sync = SyncDoc::new(1);
        sync.populate_from_document(&doc);
        let rebuilt = sync.to_document();

        assert_eq!(rebuilt.entities.len(), 3);
        assert!(rebuilt.layers.len() >= 1);

        for orig in &doc.entities {
            let rebuilt = rebuilt
                .entities
                .iter()
                .find(|e| e.id == orig.id)
                .expect(&format!("entity {} missing after roundtrip", orig.id.0));
            assert_eq!(
                crate::entity_to_bytes(orig),
                crate::entity_to_bytes(rebuilt),
                "entity {} bytes differ after roundtrip",
                orig.id.0
            );
        }
    }

    #[test]
    fn apply_mutation_returns_update() {
        let sync = SyncDoc::new(1);
        let mut doc = Document::new();

        let (result, update) = sync
            .apply_mutation(&mut doc, "addLine", r#"{"start":[0,0],"end":[50,0]}"#)
            .unwrap();

        assert!(result.contains("e_1"));
        assert!(!update.is_empty());
        assert_eq!(doc.entities.len(), 1);

        let txn = sync.doc.transact();
        assert_eq!(sync.entities.len(&txn), 1);
    }

    #[test]
    fn mutation_remove_syncs() {
        let sync = SyncDoc::new(1);
        let mut doc = Document::new();

        sync.apply_mutation(&mut doc, "addLine", r#"{"start":[0,0],"end":[50,0]}"#)
            .unwrap();
        sync.apply_mutation(&mut doc, "addLine", r#"{"start":[0,0],"end":[0,50]}"#)
            .unwrap();
        sync.apply_mutation(&mut doc, "remove", r#"{"target":"e_1"}"#)
            .unwrap();

        assert_eq!(doc.entities.len(), 1);
        let txn = sync.doc.transact();
        assert_eq!(sync.entities.len(&txn), 1);
        assert!(sync.entities.get(&txn, "e_1").is_none());
        assert!(sync.entities.get(&txn, "e_2").is_some());
    }

    #[test]
    fn two_peers_sync() {
        let server = SyncDoc::new(1);
        let mut server_doc = Document::new();
        server
            .apply_mutation(
                &mut server_doc,
                "addLine",
                r#"{"start":[0,0],"end":[100,0]}"#,
            )
            .unwrap();

        // Client starts empty, syncs from server
        let client = SyncDoc::new(2);
        let client_sv = client.state_vector();
        let update = server.encode_diff(&client_sv).unwrap();
        client.apply_update(&update).unwrap();

        let client_doc = client.to_document();
        assert_eq!(client_doc.entities.len(), 1);

        // Client adds an entity, syncs back to server
        let mut client_doc2 = client.to_document();
        let (_, update2) = client
            .apply_mutation(
                &mut client_doc2,
                "addCircle",
                r#"{"center":[50,50],"radius":10}"#,
            )
            .unwrap();
        assert!(!update2.is_empty());

        server.apply_update(&update2).unwrap();
        let txn = server.doc.transact();
        assert_eq!(server.entities.len(&txn), 2);
    }

    #[test]
    fn empty_doc_handshake() {
        // Simulate empty browser connecting to empty server
        let server = SyncDoc::new(1);
        server.populate_from_document(&Document::new());

        let client = SyncDoc::new(2);

        // Step 1: client sends SV
        let client_sv = client.state_vector();

        // Step 2: server sends diff
        let update = server.encode_diff(&client_sv).unwrap();
        // Server's encode_diff against client's raw [0] byte
        let raw_client_sv = vec![0u8];
        let diff_against_raw = server.encode_diff(&raw_client_sv).unwrap();
        println!(
            "Diff against raw [0]: {} bytes = {:?}",
            diff_against_raw.len(),
            &diff_against_raw
        );
        assert!(
            diff_against_raw.len() <= 2,
            "diff against [0] should be trivial, got {} bytes",
            diff_against_raw.len()
        );

        println!("Server diff: {} bytes = {:?}", update.len(), &update);
        client.apply_update(&update).unwrap();

        // Step 3: server sends its SV
        let server_sv = server.state_vector();

        // Step 4: client sends its diff
        let client_update = client.encode_diff(&server_sv).unwrap();
        if !client_update.is_empty() {
            server.apply_update(&client_update).unwrap();
        }

        // Both sides should be in sync (empty)
        assert_eq!(client.to_document().entities.len(), 0);
        assert_eq!(server.to_document().entities.len(), 0);
    }

    #[test]
    fn bincode_roundtrip_all_shapes() {
        let mut doc = Document::new();
        doc.add_line(
            Point::new(1.0, 2.0),
            Point::new(3.0, 4.0),
            "L",
            Some(Color::rgb(100, 200, 50)),
        );
        doc.add_circle(Point::new(5.0, 6.0), 7.0, "C", Some(Color::rgb(10, 20, 30)));
        doc.add_arc(
            Point::new(0.0, 0.0),
            50.0,
            0.0,
            1.5,
            "A",
            Some(Color::rgb(255, 0, 0)),
        );
        doc.add_polyline(
            vec![
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(1.0, 1.0),
            ],
            true,
            "P",
            Some(Color::rgb(0, 255, 0)),
        );

        for ent in &doc.entities {
            let bytes = crate::entity_to_bytes(ent);
            let rebuilt = crate::entity_from_bytes(&bytes)
                .expect(&format!("failed to roundtrip entity {}", ent.id.0));
            assert_eq!(rebuilt.id, ent.id);
            assert_eq!(rebuilt.layer, ent.layer);
            assert_eq!(rebuilt.color, ent.color);
            assert_eq!(crate::entity_to_bytes(&rebuilt), bytes);
        }
    }

    #[test]
    fn batch_produces_single_update() {
        let sync = SyncDoc::new(1);
        let mut doc = Document::new();

        // Without batch: each mutation produces an update
        let (_, u1) = sync
            .apply_mutation(&mut doc, "addLine", r#"{"start":[0,0],"end":[10,0]}"#)
            .unwrap();
        assert!(!u1.is_empty(), "non-batch mutation should produce update");

        // With batch: mutations produce no updates until end_batch
        sync.begin_batch(&doc);
        let (_, u2) = sync
            .apply_mutation(&mut doc, "addLine", r#"{"start":[0,10],"end":[10,10]}"#)
            .unwrap();
        assert!(u2.is_empty(), "batched mutation should defer update");
        let (_, u3) = sync
            .apply_mutation(&mut doc, "addCircle", r#"{"center":[5,5],"radius":2}"#)
            .unwrap();
        assert!(u3.is_empty(), "batched mutation should defer update");

        let batch_update = sync.end_batch(&doc);
        assert!(
            !batch_update.is_empty(),
            "end_batch should produce combined update"
        );

        // Apply the batch update to a second peer and verify
        let peer = SyncDoc::new(2);
        // First sync everything before batch
        let sv = peer.state_vector();
        let full = sync.encode_diff(&sv).unwrap();
        peer.apply_update(&full).unwrap();

        let peer_doc = peer.to_document();
        assert_eq!(peer_doc.entities.len(), 3);
    }

    #[test]
    fn clear_removes_layers_from_yrs() {
        let sync = SyncDoc::new(1);
        let mut doc = Document::new();

        // Add entities on two layers
        sync.apply_mutation(
            &mut doc,
            "addLine",
            r#"{"start":[0,0],"end":[10,0],"layer":"WALLS"}"#,
        )
        .unwrap();
        sync.apply_mutation(
            &mut doc,
            "addCircle",
            r#"{"center":[5,5],"radius":2,"layer":"ELEC"}"#,
        )
        .unwrap();

        {
            let txn = sync.doc.transact();
            assert!(sync.layers.get(&txn, "WALLS").is_some());
            assert!(sync.layers.get(&txn, "ELEC").is_some());
        }

        // Clear should remove all entities AND layers from Yrs
        sync.apply_mutation(&mut doc, "clear", "{}").unwrap();
        assert_eq!(doc.entities.len(), 0);
        assert_eq!(doc.layers.len(), 0);

        let txn = sync.doc.transact();
        assert_eq!(sync.entities.len(&txn), 0);
        assert_eq!(
            sync.layers.len(&txn),
            0,
            "stale layers leaked in Yrs after clear"
        );
    }
}
