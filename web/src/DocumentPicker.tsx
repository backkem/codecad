import { useEffect, useState } from "react";
import { cad, type DocumentInfo } from "./cad";

interface Props {
  open: boolean;
  onClose: () => void;
  onSelect: (docId: string) => void;
  onNewDrawing: () => void;
  openTabIds: string[];
}

export function DocumentPicker({
  open,
  onClose,
  onSelect,
  onNewDrawing,
  openTabIds,
}: Props) {
  const [docs, setDocs] = useState<DocumentInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setError(null);
    cad.api
      .listDocuments()
      .then(setDocs)
      .catch((e: Error) => setError(e.message))
      .finally(() => setLoading(false));
  }, [open]);

  if (!open) return null;

  // Group by prefix
  const grouped = new Map<string, DocumentInfo[]>();
  for (const doc of docs) {
    const prefix = doc.prefix || "(root)";
    const list = grouped.get(prefix) || [];
    list.push(doc);
    grouped.set(prefix, list);
  }

  return (
    <div className="doc-picker-overlay" onClick={onClose}>
      <div className="doc-picker" onClick={(e) => e.stopPropagation()}>
        <div className="doc-picker-header">
          <span>Open Document</span>
          <button onClick={onClose}>x</button>
        </div>
        <button
          className="doc-picker-item doc-picker-new"
          onClick={() => {
            onNewDrawing();
            onClose();
          }}
        >
          <span className="doc-picker-name">+ New Drawing</span>
          <span className="doc-picker-meta">empty, in-memory</span>
        </button>
        {loading && <div className="doc-picker-loading">Loading...</div>}
        {error && <div className="doc-picker-error">{error}</div>}
        {!loading && docs.length === 0 && !error && (
          <div className="doc-picker-empty">No .dwg files found</div>
        )}
        {[...grouped.entries()].map(([prefix, items]) => (
          <div key={prefix} className="doc-picker-group">
            {prefix !== "(root)" && (
              <div className="doc-picker-folder">{prefix}</div>
            )}
            {items.map((doc) => {
              const alreadyOpen = openTabIds.includes(doc.id);
              return (
                <button
                  key={doc.id}
                  className={`doc-picker-item${alreadyOpen ? " open" : ""}`}
                  onClick={() => {
                    onSelect(doc.id);
                    onClose();
                  }}
                >
                  <span className="doc-picker-name">{doc.filename}</span>
                  {doc.entity_count != null && (
                    <span className="doc-picker-meta">
                      {doc.entity_count} entities
                    </span>
                  )}
                  {alreadyOpen && (
                    <span className="doc-picker-badge">open</span>
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
