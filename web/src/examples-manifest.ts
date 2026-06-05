export interface Example {
  id: string;
  title: string;
  desc: string;
}

export const EXAMPLES: Example[] = [
  {
    id: "01-flange-plate",
    title: "Flange Plate",
    desc: "Parametric circular pipe flange with 8 bolt holes, center bore, dimensioned radii.",
  },
  {
    id: "02-fibonacci-spiral",
    title: "Fibonacci Spiral",
    desc: "Golden ratio construction: subdivided rectangle, quarter-circle arcs, diagonal lines.",
  },
  {
    id: "03-roof-truss",
    title: "Howe Roof Truss",
    desc: "12m span structural timber truss with Howe diagonals, chords, and dimensions.",
  },
  {
    id: "04-motor-schematic",
    title: "Motor Starter Schematic",
    desc: "3-phase DOL motor starter: circuit breaker, contactor, overload, control circuit.",
  },
  {
    id: "05-bicycle-frame",
    title: "Bicycle Frame",
    desc: "Parametric road bike diamond frame with tube offsets, wheels, seat/head angles.",
  },
  {
    id: "06-piston-section",
    title: "Piston Cross-Section",
    desc: "Half-section piston and connecting rod with ring grooves, hatching, mirror.",
  },
];

/** Resolve DWG URL for an example. Base path depends on deployment. */
export function exampleDwgUrl(id: string): string {
  // On gh-pages: /examples/01-flange-plate.dwg
  // Dev / server: /examples/01-flange-plate.dwg
  return `${import.meta.env.BASE_URL}examples/${id}.dwg`;
}

/** Resolve preview image URL for an example. */
export function examplePngUrl(id: string): string {
  return `${import.meta.env.BASE_URL}examples/${id}.png`;
}
