import { useEffect, useRef, useState } from "react";
import { cad, type DocumentInfo, isServerAvailable } from "./cad";
import { ExamplesBrowser } from "./ExamplesBrowser";

interface Props {
  open: boolean;
  onClose: () => void;
  onSelect: (docId: string) => void;
  onNewDrawing: () => void;
  onLoadFile: (name: string, bytes: Uint8Array) => void;
  onLoadExample: (exampleId: string) => void;
  openTabIds: string[];
}

export function DocumentPicker({
  open,
  onClose,
  onSelect,
  onNewDrawing,
  onLoadFile,
  onLoadExample,
  openTabIds,
}: Props) {
  const [docs, setDocs] = useState<DocumentInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const server = isServerAvailable();

  useEffect(() => {
    if (!open || !server) return;
    setLoading(true);
    setError(null);
    cad.api
      .listDocuments()
      .then(setDocs)
      .catch((e: Error) => setError(e.message))
      .finally(() => setLoading(false));
  }, [open, server]);

  if (!open) return null;

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    file.arrayBuffer().then((buf) => {
      onLoadFile(file.name, new Uint8Array(buf));
      onClose();
    });
  };

  // Group server docs by prefix
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
          onClick={() => { onNewDrawing(); onClose(); }}
        >
          <span className="doc-picker-name">+ New Drawing</span>
          <span className="doc-picker-meta">empty, in-memory</span>
        </button>

        <button
          className="doc-picker-item doc-picker-new"
          onClick={() => fileRef.current?.click()}
        >
          <span className="doc-picker-name">Open file...</span>
          <span className="doc-picker-meta">.dwg from disk</span>
        </button>
        <input
          ref={fileRef}
          type="file"
          accept=".dwg"
          style={{ display: "none" }}
          onChange={handleFileChange}
        />

        {/* Server documents (when connected) */}
        {server && loading && <div className="doc-picker-loading">Loading...</div>}
        {server && error && <div className="doc-picker-error">{error}</div>}
        {server && !loading && docs.length === 0 && !error && (
          <div className="doc-picker-empty">No .dwg files on server</div>
        )}
        {server && [...grouped.entries()].map(([prefix, items]) => (
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
                  onClick={() => { onSelect(doc.id); onClose(); }}
                >
                  <span className="doc-picker-name">{doc.filename}</span>
                  {doc.entity_count != null && (
                    <span className="doc-picker-meta">{doc.entity_count} entities</span>
                  )}
                  {alreadyOpen && <span className="doc-picker-badge">open</span>}
                </button>
              );
            })}
          </div>
        ))}

        {/* Examples (always shown) */}
        <div className="doc-picker-section-label">Examples</div>
        <ExamplesBrowser onSelect={(id) => { onLoadExample(id); onClose(); }} />
      </div>
    </div>
  );
}
