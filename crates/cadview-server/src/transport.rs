//! WebTransport session handling.
//!
//! Each connected browser gets:
//! - A long-lived bidi stream for Yrs sync updates (per document)
//! - Per-call bidi streams for RPC (server-only / client-only commands)
//!
//! Phase 5 will add typed hello frames for document-scoped streams.
//! For now: first bidi stream = Yrs sync for the first loaded document,
//! subsequent streams = RPC (old protocol, backward compatible).

use crate::registry::DocumentRegistry;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use wtransport::Connection;

/// Session handler using DocumentRegistry.
///
/// Accepts bidi streams in a loop. Each stream is classified by its first
/// message: if it parses as JSON with a "method" field, it's an RPC call.
/// Otherwise it's a Yrs sync stream (binary state vector).
///
/// Sync streams include a doc_id header (first 2 bytes = key length,
/// then UTF-8 key, then the SV). But for backward compat with the old
/// protocol (raw SV, no doc_id header), we fall back to the first
/// loaded document if no header is present.
pub async fn handle_session_v2(
    conn: Connection,
    registry: Arc<Mutex<DocumentRegistry>>,
    sandbox: Arc<cadview_sandbox::Sandbox>,
) {
    tracing::info!("WebTransport session connected");

    // Determine default doc key (first available, for backward compat)
    let default_doc_key = {
        let reg = registry.lock().await;
        let available = reg.list_available();
        available.first().map(|d| d.id.clone()).unwrap_or_default()
    };

    // Accept all bidi streams and classify them
    loop {
        let (mut send, mut recv) = match conn.accept_bi().await {
            Ok(streams) => streams,
            Err(e) => {
                tracing::debug!("Session ended: {e}");
                break;
            }
        };

        let reg = registry.clone();
        let sb = sandbox.clone();
        let default_key = default_doc_key.clone();

        tokio::spawn(async move {
            // Read the first message to classify the stream
            let first_msg = match read_message(&mut recv).await {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!("Stream read error: {e}");
                    return;
                }
            };

            // Try to parse as JSON RPC
            if let Ok(req) = serde_json::from_slice::<serde_json::Value>(&first_msg) {
                if req.get("method").is_some() {
                    // It's an RPC call
                    let response = handle_rpc_call(&first_msg, reg, &default_key, &sb).await;
                    let _ = write_message(&mut send, response.as_bytes()).await;
                    return;
                }
            }

            // Not an RPC. Check if it's a sync header (JSON with "type":"document")
            // or a raw SV (old protocol backward compat).
            let (doc_key, client_sv) =
                if let Ok(header) = serde_json::from_slice::<serde_json::Value>(&first_msg) {
                    if header.get("type").and_then(|t| t.as_str()) == Some("document") {
                        let id = header["id"].as_str().unwrap_or(&default_key).to_string();
                        // Read the actual SV as the next message
                        let sv = match read_message(&mut recv).await {
                            Ok(sv) => sv,
                            Err(e) => {
                                tracing::warn!("Failed to read SV: {e}");
                                return;
                            }
                        };
                        (id, sv)
                    } else {
                        // Unknown JSON, treat first_msg as raw SV
                        (default_key.clone(), first_msg)
                    }
                } else {
                    // Binary data = raw SV (old protocol)
                    (default_key.clone(), first_msg)
                };

            // Ensure document slot exists (creates empty slot for new docs)
            {
                let mut r = reg.lock().await;
                if let Err(e) = r.get_or_load(&doc_key) {
                    tracing::error!("Failed to load/create '{}': {e}", doc_key);
                    return;
                }
            }

            tracing::info!("Yrs sync stream for '{}'", doc_key);
            if let Err(e) = run_yrs_sync_with_first_msg(send, recv, client_sv, reg, &doc_key).await
            {
                tracing::warn!("Yrs sync ended for '{}': {e}", doc_key);
            }
        });
    }

    tracing::info!("WebTransport session disconnected");
}

/// Long-lived Yrs sync where the first message (client SV) was already read.
async fn run_yrs_sync_with_first_msg(
    mut send: wtransport::SendStream,
    recv: wtransport::RecvStream,
    client_sv: Vec<u8>,
    registry: Arc<Mutex<DocumentRegistry>>,
    doc_key: &str,
) -> anyhow::Result<()> {
    let mut recv = recv;
    tracing::debug!(
        "Received client state vector ({} bytes) for '{}'",
        client_sv.len(),
        doc_key
    );

    // Step 2: Send missing updates to client
    let (update, update_tx) = {
        let reg = registry.lock().await;
        let slot = reg
            .get(doc_key)
            .ok_or_else(|| anyhow::anyhow!("doc '{}' not loaded", doc_key))?;
        let update = slot.sync_doc.encode_diff(&client_sv)?;
        let tx = slot.update_tx.clone();
        (update, tx)
    };
    write_message(&mut send, &update).await?;

    // Step 3: Send our state vector
    let our_sv = {
        let reg = registry.lock().await;
        let slot = reg.get(doc_key).unwrap();
        slot.sync_doc.state_vector()
    };
    write_message(&mut send, &our_sv).await?;

    // Step 4: Read client's updates
    let client_update = read_message(&mut recv).await?;
    if !client_update.is_empty() {
        let mut reg = registry.lock().await;
        let slot = reg.get_mut(doc_key).unwrap();
        slot.sync_doc.apply_update(&client_update)?;
        let rebuilt = slot.sync_doc.to_document();
        slot.document = rebuilt;
        tracing::debug!(
            "Applied client->server update ({} bytes) for '{}'",
            client_update.len(),
            doc_key
        );
    }

    tracing::info!("Yrs initial sync complete for '{}'", doc_key);

    // Increment client count
    {
        let mut reg = registry.lock().await;
        if let Some(slot) = reg.get_mut(doc_key) {
            slot.client_count += 1;
        }
    }

    let mut update_rx = update_tx.subscribe();

    // Step 5: Continuous bidirectional
    let result = loop {
        tokio::select! {
            msg = read_message(&mut recv) => {
                match msg {
                    Ok(update) if !update.is_empty() => {
                        let mut reg = registry.lock().await;
                        let slot = reg.get_mut(doc_key).unwrap();
                        slot.sync_doc.apply_update(&update)?;
                        let rebuilt = slot.sync_doc.to_document();
                        let count = rebuilt.entities.len();
                        slot.document = rebuilt;
                        let tx = slot.update_tx.clone();
                        drop(reg);
                        let _ = tx.send(update.clone());
                        tracing::info!("Applied client update for '{}' ({} bytes, {} entities)",
                            doc_key, update.len(), count);
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        tracing::debug!("Yrs sync stream closed for '{}': {e}", doc_key);
                        break Ok(());
                    }
                }
            }
            update = update_rx.recv() => {
                match update {
                    Ok(bytes) if !bytes.is_empty() => {
                        write_message(&mut send, &bytes).await?;
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("Client lagged {n} updates for '{}', sending full state", doc_key);
                        let full = {
                            let reg = registry.lock().await;
                            let slot = reg.get(doc_key).unwrap();
                            slot.sync_doc.encode_state()
                        };
                        write_message(&mut send, &full).await?;
                    }
                    Err(_) => break Ok(()),
                }
            }
        }
    };

    // Decrement client count
    {
        let mut reg = registry.lock().await;
        reg.release(doc_key);
    }

    result
}

async fn handle_rpc_call(
    msg: &[u8],
    registry: Arc<Mutex<DocumentRegistry>>,
    default_doc_key: &str,
    sandbox: &Arc<cadview_sandbox::Sandbox>,
) -> String {
    let Ok(req) = serde_json::from_slice::<serde_json::Value>(msg) else {
        return r#"{"error":"invalid JSON"}"#.to_string();
    };

    let method = req["method"].as_str().unwrap_or("");
    let args = req
        .get("args")
        .map(|a| a.to_string())
        .unwrap_or_else(|| "{}".to_string());

    // Use doc_id from request if provided, otherwise fall back to default
    let doc_key = req["doc_id"].as_str().unwrap_or(default_doc_key);

    match method {
        "save" => {
            let path = req["args"]["path"]
                .as_str()
                .unwrap_or("cadview-output.json");
            let reg = registry.lock().await;
            let Some(slot) = reg.get(doc_key) else {
                return format!(r#"{{"error":"document '{}' not loaded"}}"#, doc_key);
            };
            let entities: Vec<cadview_core::EntityJson> = slot
                .document
                .entities
                .iter()
                .map(|e| e.to_json(&slot.document))
                .collect();
            match serde_json::to_string_pretty(&entities) {
                Ok(json) => match std::fs::write(path, &json) {
                    Ok(()) => {
                        tracing::info!("Saved {} entities to {path}", entities.len());
                        format!(
                            r#"{{"ok":true,"path":"{path}","entities":{}}}"#,
                            entities.len()
                        )
                    }
                    Err(e) => format!(r#"{{"error":"write failed: {e}"}}"#),
                },
                Err(e) => format!(r#"{{"error":"serialize failed: {e}"}}"#),
            }
        }
        "saveDwg" => {
            let path = req["args"]["path"].as_str().unwrap_or("output.dwg");
            let reg = registry.lock().await;
            let Some(slot) = reg.get(doc_key) else {
                return format!(r#"{{"error":"document '{}' not loaded"}}"#, doc_key);
            };
            let result = if let Some(ref src) = slot.source_dwg_path {
                // Overlay mode: load original DWG, add new layers/entities, save.
                // Produces DWGs that AutoCAD accepts.
                let prefixes = &["E_"];
                tracing::info!("Overlay DWG save: base={src}, overlay prefixes={prefixes:?}");
                cadview_core::save_dwg_overlay(&slot.document, src, path, prefixes)
            } else {
                cadview_core::save_dwg(&slot.document, path)
            };
            match result {
                Ok(()) => {
                    tracing::info!(
                        "Saved DWG to {path} ({} entities)",
                        slot.document.entities.len()
                    );
                    format!(
                        r#"{{"ok":true,"path":"{path}","entities":{}}}"#,
                        slot.document.entities.len()
                    )
                }
                Err(e) => format!(r#"{{"error":"dwg write failed: {e}"}}"#),
            }
        }
        "savePdf" => {
            let path = req["args"]["path"].as_str().unwrap_or("output.pdf");
            let reg = registry.lock().await;
            let Some(slot) = reg.get(doc_key) else {
                return format!(r#"{{"error":"document '{}' not loaded"}}"#, doc_key);
            };
            let opts = cadview_core::pdf::PdfOptions::default();
            let bytes = cadview_core::export_pdf_bytes(&slot.document, &opts);
            match std::fs::write(path, &bytes) {
                Ok(()) => {
                    tracing::info!("Saved PDF to {path} ({} bytes)", bytes.len());
                    format!(r#"{{"ok":true,"path":"{path}","bytes":{}}}"#, bytes.len())
                }
                Err(e) => format!(r#"{{"error":"pdf write failed: {e}"}}"#),
            }
        }
        "loadDwg" => {
            let path = req["args"]["path"].as_str().unwrap_or("");
            if path.is_empty() {
                return r#"{"error":"path required"}"#.to_string();
            }
            match cadview_core::load_dwg(path) {
                Ok(new_doc) => {
                    let count = new_doc.entities.len();
                    let mut reg = registry.lock().await;
                    let slot = reg.get_mut(doc_key).unwrap();

                    let sv_before = slot.sync_doc.state_vector();
                    slot.sync_doc.populate_from_document(&new_doc);
                    let update = slot.sync_doc.encode_diff(&sv_before).unwrap_or_default();
                    slot.document = new_doc;

                    if !update.is_empty() {
                        let _ = slot.update_tx.send(update);
                    }

                    tracing::info!("Loaded DWG {path}: {count} entities");
                    format!(r#"{{"ok":true,"entities":{count}}}"#)
                }
                Err(e) => format!(r#"{{"error":"load failed: {e}"}}"#),
            }
        }
        "beginBatch" => {
            let reg = registry.lock().await;
            let Some(slot) = reg.get(doc_key) else {
                return format!(r#"{{"error":"document '{}' not loaded"}}"#, doc_key);
            };
            slot.sync_doc.begin_batch(&slot.document);
            r#"{"ok":true}"#.to_string()
        }
        "endBatch" => {
            let reg = registry.lock().await;
            let Some(slot) = reg.get(doc_key) else {
                return format!(r#"{{"error":"document '{}' not loaded"}}"#, doc_key);
            };
            let update = slot.sync_doc.end_batch(&slot.document);
            if !update.is_empty() {
                let _ = slot.update_tx.send(update);
            }
            r#"{"ok":true}"#.to_string()
        }
        "runScript" => {
            let program = req["args"]["program"].as_str().unwrap_or("");
            let base_dir = std::env::current_dir().unwrap_or_default();
            match crate::script::run_script(sandbox, &registry, doc_key, program, &base_dir, None)
                .await
            {
                Ok(output) => serde_json::json!({
                    "ok": true,
                    "value": output.value,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                })
                .to_string(),
                Err(e) => format!(r#"{{"error":{}}}"#, serde_json::json!(e)),
            }
        }
        "exec" => {
            let path = req["args"]["path"].as_str().unwrap_or("");
            match crate::script::exec_file(sandbox, &registry, doc_key, path, None).await {
                Ok(output) => serde_json::json!({
                    "ok": true,
                    "value": output.value,
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                })
                .to_string(),
                Err(e) => format!(r#"{{"error":{}}}"#, serde_json::json!(e)),
            }
        }
        _ => {
            // Generic document mutation
            let mut reg = registry.lock().await;
            let Some(slot) = reg.get_mut(doc_key) else {
                return format!(r#"{{"error":"document '{}' not loaded"}}"#, doc_key);
            };
            let (result, update) =
                match slot
                    .sync_doc
                    .apply_mutation(&mut slot.document, method, &args)
                {
                    Ok(r) => r,
                    Err(e) => return format!(r#"{{"error":{}}}"#, serde_json::json!(e)),
                };

            if !update.is_empty() {
                let _ = slot.update_tx.send(update);
            }

            result
        }
    }
}

// ── Framing helpers ───────────────────────────────────────────────────

async fn write_message(send: &mut wtransport::SendStream, data: &[u8]) -> anyhow::Result<()> {
    let mut frame = Vec::with_capacity(4 + data.len());
    frame.extend_from_slice(&(data.len() as u32).to_be_bytes());
    frame.extend_from_slice(data);
    send.write_all(&frame)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

async fn read_message(recv: &mut wtransport::RecvStream) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    recv.read_exact(&mut len_buf)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > 64 * 1024 * 1024 {
        anyhow::bail!("message too large: {len} bytes");
    }
    let mut buf = vec![0u8; len];
    recv.read_exact(&mut buf)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(buf)
}
