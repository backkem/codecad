import { useCallback, useEffect, useRef } from "react";

interface LayerState {
  name: string;
  color: [number, number, number];
  visible: boolean;
  entityCount: number;
}

interface Props {
  layers: LayerState[];
  open: boolean;
  onTogglePanel: () => void;
  onToggleLayer: (name: string) => void;
}

/**
 * Photoshop-style drag-toggle: mousedown on a visibility checkbox sets
 * the "paint" target (the opposite of that layer's current state), then
 * dragging across other checkboxes applies the same state to each one
 * entered, without re-toggling layers already changed in this gesture.
 */
export function LayerPanel({
  layers,
  open,
  onTogglePanel,
  onToggleLayer,
}: Props) {
  // drag state lives in refs so we don't re-render mid-gesture
  const dragging = useRef(false);
  const paintTarget = useRef<boolean>(true); // what visibility to paint
  const painted = useRef<Set<string>>(new Set()); // layers already set this gesture

  const startDrag = useCallback(
    (name: string, currentVisible: boolean) => {
      dragging.current = true;
      paintTarget.current = !currentVisible;
      painted.current = new Set([name]);
      onToggleLayer(name);
    },
    [onToggleLayer],
  );

  const enterRow = useCallback(
    (name: string, currentVisible: boolean) => {
      if (!dragging.current) return;
      if (painted.current.has(name)) return;
      if (currentVisible === paintTarget.current) return; // already correct
      painted.current.add(name);
      onToggleLayer(name);
    },
    [onToggleLayer],
  );

  // Global mouseup to end the drag wherever it ends
  useEffect(() => {
    const up = () => {
      dragging.current = false;
    };
    window.addEventListener("mouseup", up);
    return () => window.removeEventListener("mouseup", up);
  }, []);

  return (
    <div className="layer-panel" data-open={open}>
      <button className="layer-toggle" onClick={onTogglePanel}>
        {open ? "\u00d7" : "Layers"}
      </button>
      {open && (
        <div className="layer-list">
          <div className="layer-header">Layers</div>
          {layers.map((l) => (
            <div
              key={l.name}
              className="layer-row"
              data-visible={l.visible}
              onMouseEnter={() => enterRow(l.name, l.visible)}
            >
              <span
                className="layer-swatch"
                style={{
                  backgroundColor: `rgb(${l.color[0]},${l.color[1]},${l.color[2]})`,
                  opacity: l.visible ? 1 : 0.3,
                }}
              />
              <input
                type="checkbox"
                checked={l.visible}
                readOnly
                onMouseDown={(e) => {
                  e.preventDefault(); // prevent native checkbox toggle + text selection
                  startDrag(l.name, l.visible);
                }}
              />
              <span className="layer-name">{l.name}</span>
              <span className="layer-count">{l.entityCount}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
