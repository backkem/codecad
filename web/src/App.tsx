import { type DragEvent, useCallback, useEffect, useState } from "react";
import { cad, type EntityJson, getInitialSessionId, onTabRenamed } from "./cad";
import { DocumentPicker } from "./DocumentPicker";
import { LayerPanel } from "./LayerPanel";
import { TabBar } from "./TabBar";
import { ViewportContainer } from "./ViewportContainer";

interface Tab {
  id: string;
  label: string;
}

interface LayerState {
  name: string;
  color: [number, number, number];
  visible: boolean;
  entityCount: number;
}

let _newDrawingCounter = 0;

export function App() {
  const initId = getInitialSessionId();
  const initLabel =
    initId
      .replace(/\.dwg$/i, "")
      .split("/")
      .pop() || initId;
  const [tabs, setTabs] = useState<Tab[]>([{ id: initId, label: initLabel }]);
  const [activeTabId, setActiveTabId] = useState<string>(initId);
  const [layers, setLayers] = useState<LayerState[]>([]);
  const [panelOpen, setPanelOpen] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [dragOver, setDragOver] = useState(false);

  // Register tab rename callback (called by cad.save/saveDwg)
  useEffect(() => {
    onTabRenamed((sessionId, newLabel) => {
      setTabs((prev) =>
        prev.map((t) => (t.id === sessionId ? { ...t, label: newLabel } : t)),
      );
    });
  }, []);

  // Switch active tab
  function activateTab(id: string) {
    setActiveTabId(id);
    try {
      cad.useSession(id);
    } catch {
      // Session may not exist yet
    }
  }

  // Open a server document
  function openDocument(docId: string) {
    const label =
      docId
        .replace(/\.dwg$/i, "")
        .split("/")
        .pop() || docId;
    let sessionId = docId;
    let suffix = 1;
    while (tabs.some((t) => t.id === sessionId)) {
      suffix++;
      sessionId = `${docId}#${suffix}`;
    }
    const displayLabel = suffix > 1 ? `${label} (${suffix})` : label;
    try {
      cad.sessions.create(sessionId);
    } catch (e) {
      console.warn("Failed to create session:", e);
      return;
    }
    setTabs((prev) => [...prev, { id: sessionId, label: displayLabel }]);
    activateTab(sessionId);
  }

  // New empty drawing
  function newDrawing() {
    _newDrawingCounter++;
    const id = `new-${Date.now()}-${_newDrawingCounter}`;
    const label = `untitled-${_newDrawingCounter}`;
    try {
      cad.sessions.create(id);
    } catch (e) {
      console.warn("Failed to create new drawing:", e);
      return;
    }
    setTabs((prev) => [...prev, { id, label }]);
    activateTab(id);
  }

  // Load a dropped DWG file into a new tab
  function loadDroppedFile(file: File) {
    const label = file.name.replace(/\.dwg$/i, "");
    const id = `drop-${Date.now()}-${file.name}`;
    try {
      cad.sessions.create(id);
    } catch (e) {
      console.warn("Failed to create session for dropped file:", e);
      return;
    }
    setTabs((prev) => [...prev, { id, label }]);
    activateTab(id);

    // Read the file and load DWG bytes into the session
    file.arrayBuffer().then((buf) => {
      try {
        const result = cad.sessions.loadDwgBytes(id, new Uint8Array(buf));
        console.log(
          `[CodeCAD] Loaded ${file.name}: ${result.entities} entities`,
        );
      } catch (e) {
        console.error(`[CodeCAD] Failed to parse ${file.name}:`, e);
      }
    });
  }

  // Close a tab
  function closeTab(id: string) {
    try {
      cad.sessions.destroy(id);
    } catch {
      // Already destroyed
    }
    setTabs((prev) => {
      const next = prev.filter((t) => t.id !== id);
      if (activeTabId === id && next.length > 0) {
        activateTab(next[next.length - 1].id);
      }
      return next;
    });
  }

  // Drag and drop handlers
  const onDragOver = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.dataTransfer.types.includes("Files")) {
      setDragOver(true);
    }
  }, []);

  const onDragLeave = useCallback((e: DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
  }, []);

  const onDrop = useCallback(
    (e: DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setDragOver(false);
      const files = Array.from(e.dataTransfer.files);
      for (const file of files) {
        if (file.name.toLowerCase().endsWith(".dwg")) {
          loadDroppedFile(file);
        } else {
          console.warn(`[CodeCAD] Ignoring non-DWG file: ${file.name}`);
        }
      }
    },
    [tabs],
  );

  // Poll layers from the active session
  useEffect(() => {
    const poll = setInterval(() => {
      try {
        const entities = cad.entities() as EntityJson[];
        const layerMap = new Map<
          string,
          { color: [number, number, number]; count: number }
        >();
        for (const e of entities) {
          const existing = layerMap.get(e.layer);
          if (existing) {
            existing.count++;
          } else {
            layerMap.set(e.layer, { color: e.color, count: 1 });
          }
        }
        setLayers((prev) => {
          const next: LayerState[] = [];
          for (const [name, info] of layerMap) {
            const existing = prev.find((l) => l.name === name);
            next.push({
              name,
              color: info.color,
              visible: existing?.visible ?? true,
              entityCount: info.count,
            });
          }
          next.sort((a, b) => a.name.localeCompare(b.name));
          return next;
        });
      } catch {
        // WASM not ready yet
      }
    }, 1000);
    return () => clearInterval(poll);
  }, [activeTabId]);

  const activeTab = tabs.find((t) => t.id === activeTabId);

  return (
    <div
      className="app-root"
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <TabBar
        tabs={tabs}
        activeId={activeTabId}
        onSelect={activateTab}
        onClose={closeTab}
        onAdd={() => setPickerOpen(true)}
      />

      <div className="viewport-area">
        {activeTab && (
          <ViewportContainer
            key={activeTab.id}
            sessionId={activeTab.id}
            focused={true}
            onFocus={() => {}}
          />
        )}
        {dragOver && (
          <div className="drop-overlay">
            <div className="drop-message">Drop .dwg file to open</div>
          </div>
        )}

        {layers.length > 0 && (
          <LayerPanel
            layers={layers}
            open={panelOpen}
            onTogglePanel={() => setPanelOpen(!panelOpen)}
            onToggleLayer={(name) => {
              setLayers((prev) =>
                prev.map((l) => {
                  if (l.name !== name) return l;
                  return { ...l, visible: !l.visible };
                }),
              );
              const cur = layers.find((l) => l.name === name);
              cad.setLayerVisible(name, !(cur?.visible ?? true));
            }}
          />
        )}
      </div>

      <DocumentPicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onSelect={openDocument}
        onNewDrawing={newDrawing}
        openTabIds={tabs.map((t) => t.id)}
      />
    </div>
  );
}
