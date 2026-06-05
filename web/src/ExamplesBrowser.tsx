import { EXAMPLES, examplePngUrl } from "./examples-manifest";

interface Props {
  onSelect: (exampleId: string) => void;
}

export function ExamplesBrowser({ onSelect }: Props) {
  return (
    <div className="examples-grid">
      {EXAMPLES.map((ex) => (
        <button
          key={ex.id}
          className="example-card"
          onClick={() => onSelect(ex.id)}
        >
          <img src={examplePngUrl(ex.id)} alt={ex.title} loading="lazy" />
          <div className="example-card-title">{ex.title}</div>
          <div className="example-card-desc">{ex.desc}</div>
        </button>
      ))}
    </div>
  );
}
