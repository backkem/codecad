import { useEffect, useRef } from "react";
import { cad } from "./cad";

interface Props {
  sessionId: string;
  focused: boolean;
  onFocus: () => void;
}

let canvasCounter = 0;

export function ViewportContainer({ sessionId, focused, onFocus }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const canvasIdRef = useRef<string>("");

  useEffect(() => {
    const canvasId = `cv_${++canvasCounter}`;
    canvasIdRef.current = canvasId;

    const canvas = canvasRef.current;
    if (!canvas) return;
    canvas.id = canvasId;

    // Start the eframe renderer for this session
    try {
      cad.viewport.start(canvasId, sessionId);
    } catch (e) {
      console.error(`[CodeCAD] Failed to start renderer for ${sessionId}:`, e);
    }

    return () => {
      try {
        cad.viewport.stop(canvasIdRef.current);
      } catch {
        // Session may already be destroyed
      }
    };
  }, [sessionId]);

  return (
    <div
      className={`viewport-container${focused ? " focused" : ""}`}
      onClick={onFocus}
    >
      <canvas ref={canvasRef} />
    </div>
  );
}
