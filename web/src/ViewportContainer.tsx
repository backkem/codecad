import { useEffect, useRef, useState } from "react";
import {
  cad,
  hasWebGpu,
  isServerAvailable,
  NO_WEBGPU_3D,
  type View3dStats,
} from "./cad";

interface Props {
  sessionId: string;
  focused: boolean;
  onFocus: () => void;
  /** Open in 3D (a tab made from a prebaked scene). */
  initial3d?: boolean;
}

type Quality = "working" | "presentation";

const VIEWS = [
  ["iso", "Iso"],
  ["iso-sw", "Iso SW"],
  ["iso-ne", "Iso NE"],
  ["iso-nw", "Iso NW"],
  ["front", "Front"],
  ["back", "Back"],
  ["left", "Left"],
  ["right", "Right"],
  ["top", "Top"],
  ["bottom", "Bottom"],
] as const;

/** Session facts the toolbar needs, polled. */
function useSessionInfo(sessionId: string) {
  const [info, setInfo] = useState({ parts: 0, baked: false });
  useEffect(() => {
    const poll = () => {
      const s = cad.sessions.list().find((x) => x.id === sessionId);
      if (s) {
        setInfo((prev) =>
          prev.parts === s.part_count && prev.baked === s.baked
            ? prev
            : { parts: s.part_count, baked: s.baked },
        );
      }
    };
    poll();
    const t = setInterval(poll, 1000);
    return () => clearInterval(t);
  }, [sessionId]);
  return info;
}

export function ViewportContainer({
  sessionId,
  focused,
  onFocus,
  initial3d = false,
}: Props) {
  const [mode, setMode] = useState<"2d" | "3d">(initial3d ? "3d" : "2d");
  const info = useSessionInfo(sessionId);
  const show3dToggle = info.parts > 0 || info.baked || mode === "3d";

  return (
    <div
      className={`viewport-container${focused ? " focused" : ""}`}
      onClick={onFocus}
    >
      {mode === "2d" ? (
        <View2d sessionId={sessionId} />
      ) : (
        <View3d sessionId={sessionId} baked={info.baked} />
      )}
      {show3dToggle && (
        <div className="view-toolbar view-toolbar-mode">
          <Segmented
            value={mode}
            options={[
              ["2d", "2D"],
              ["3d", "3D"],
            ]}
            onChange={(m) => setMode(m)}
          />
        </div>
      )}
    </div>
  );
}

function View2d({ sessionId }: { sessionId: string }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let key = "";
    try {
      key = cad.viewport.start(canvas, sessionId);
    } catch (e) {
      console.error(`[CodeCAD] Failed to start renderer for ${sessionId}:`, e);
    }
    return () => {
      try {
        cad.viewport.stop(key);
      } catch {
        // Session may already be destroyed
      }
    };
  }, [sessionId]);
  return <canvas ref={canvasRef} key="2d" />;
}

/**
 * The 3D view of a session's parts. Geometry comes from the session (a
 * prebaked scene it holds, or its own kernel); with a server and no bake
 * yet, the server's bake of the document is fetched (interim, until the
 * page evaluates parts itself) and fetched again when the parts change.
 */
function View3d({ sessionId, baked }: { sessionId: string; baked: boolean }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const keyRef = useRef("");
  // Whether the session had a bake when the view started (no fetch then).
  const bakedAtStart = useRef(baked);
  const [quality, setQuality] = useState<Quality>("working");
  const [projection, setProjection] = useState<"orthographic" | "perspective">(
    "orthographic",
  );
  const [stats, setStats] = useState<View3dStats | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let stopped = false;
    let fetching = false;
    let bakedPartsSig = "";
    const partsSig = () => {
      const s = cad.sessions.list().find((x) => x.id === sessionId);
      return s ? `${s.part_count}` : "";
    };
    const fetchBake = async () => {
      if (fetching) return;
      if (!isServerAvailable()) {
        // No server to bake: the page evaluates the parts itself.
        cad.loadKernel().catch((e) => setMessage(`${e}`));
        return;
      }
      fetching = true;
      try {
        const bytes = await cad.api.fetchBake(sessionId);
        if (stopped) return;
        cad.sessions.loadBake(sessionId, bytes, false);
        bakedPartsSig = partsSig();
        console.log(
          `[CodeCAD] 3D: prebaked scene from the server, ${bytes.byteLength} bytes`,
        );
      } catch (e) {
        setMessage(`${e}`);
      } finally {
        fetching = false;
      }
    };
    let poll: ReturnType<typeof setInterval> | null = null;
    (async () => {
      if (!(await hasWebGpu())) {
        setMessage(NO_WEBGPU_3D);
        return;
      }
      if (stopped) return;
      try {
        keyRef.current = cad.view3d.start(canvas, sessionId, {
          quality: "working",
        });
      } catch (e) {
        setMessage(`${e}`);
        return;
      }
      // Geometry: a bake the session holds, else the server's bake of the
      // document, else the page's own kernel (fetched now, once).
      if (!bakedAtStart.current) await fetchBake();
      poll = setInterval(() => {
        if (!keyRef.current) return;
        let s: View3dStats;
        try {
          s = cad.view3d.stats(keyRef.current);
        } catch {
          return;
        }
        setStats(s);
        // The parts changed since the bake, or the bake cannot answer.
        const needs =
          s.error?.includes("prebaked scene does not hold") ||
          s.error?.includes("no prebaked scene") ||
          (bakedPartsSig !== "" && partsSig() !== bakedPartsSig);
        if (needs) fetchBake();
      }, 250);
    })();
    return () => {
      stopped = true;
      if (poll) clearInterval(poll);
      if (keyRef.current) {
        try {
          cad.view3d.stop(keyRef.current);
        } catch {
          // Session may already be destroyed
        }
        keyRef.current = "";
      }
    };
  }, [sessionId]);

  const command = (c: Parameters<typeof cad.view3d.command>[1]) => {
    if (!keyRef.current) return;
    try {
      cad.view3d.command(keyRef.current, c);
    } catch (e) {
      console.warn("[CodeCAD] 3D view:", e);
    }
  };

  const error = message ?? stats?.error ?? null;
  return (
    <>
      <canvas ref={canvasRef} key="3d" className="view3d-canvas" />
      {!message && (
        <div className="view-toolbar view-toolbar-3d">
          <Segmented
            value={quality}
            options={[
              ["working", "Working"],
              ["presentation", "Presentation"],
            ]}
            onChange={(q) => {
              setQuality(q);
              command({ quality: q });
            }}
          />
          <select
            className="view-select"
            defaultValue="iso"
            onChange={(e) =>
              command({
                view: e.target.value as (typeof VIEWS)[number][0],
              })
            }
            aria-label="Named view"
          >
            {VIEWS.map(([v, label]) => (
              <option key={v} value={v}>
                {label}
              </option>
            ))}
          </select>
          <Segmented
            value={projection}
            options={[
              ["orthographic", "Ortho"],
              ["perspective", "Persp"],
            ]}
            onChange={(p) => {
              setProjection(p);
              command({ projection: p });
            }}
          />
          <button
            type="button"
            className="view-button"
            onClick={() => command({ fit: true })}
          >
            Fit
          </button>
        </div>
      )}
      {stats && quality === "presentation" && !error && (
        <div className="view-status">
          {stats.samples} / {stats.target} spp
          {stats.traceScale > 1 ? " · half resolution" : ""}
        </div>
      )}
      {error && !stats?.frames && <div className="view-message">{error}</div>}
    </>
  );
}

function Segmented<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: readonly (readonly [T, string])[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="segmented" role="radiogroup">
      {options.map(([v, label]) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={v === value}
          className={v === value ? "active" : ""}
          onClick={() => onChange(v)}
        >
          {label}
        </button>
      ))}
    </div>
  );
}
