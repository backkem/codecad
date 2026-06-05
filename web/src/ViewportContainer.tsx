import { useEffect, useRef } from "react";
import { cad } from "./cad";

interface Props {
  sessionId: string;
  focused: boolean;
  onFocus: () => void;
}

export function ViewportContainer({ sessionId, focused, onFocus }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererKeyRef = useRef<string>("");

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    try {
      rendererKeyRef.current = cad.viewport.start(canvas, sessionId);
    } catch (e) {
      console.error(`[CodeCAD] Failed to start renderer for ${sessionId}:`, e);
    }

    return () => {
      try {
        cad.viewport.stop(rendererKeyRef.current);
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
