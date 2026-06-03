interface Tab {
  id: string;
  label: string;
}

interface Props {
  tabs: Tab[];
  activeId: string | null;
  onSelect: (id: string) => void;
  onClose: (id: string) => void;
  onAdd: () => void;
}

export function TabBar({ tabs, activeId, onSelect, onClose, onAdd }: Props) {
  return (
    <div className="tab-bar">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className={`tab${tab.id === activeId ? " active" : ""}`}
          onClick={() => onSelect(tab.id)}
        >
          <span className="tab-label">{tab.label}</span>
          <button
            className="tab-close"
            onClick={(e) => {
              e.stopPropagation();
              onClose(tab.id);
            }}
          >
            x
          </button>
        </div>
      ))}
      <button className="tab-add" onClick={onAdd}>
        +
      </button>
    </div>
  );
}
