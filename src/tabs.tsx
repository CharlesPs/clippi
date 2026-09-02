import React from "react";

type TabValue = string;

type TabItem<T extends TabValue> = {
  value: T;
  label: string;
  badge?: React.ReactNode;
  content: React.ReactNode;
};

type TabsProps<T extends TabValue> = {
  value: T;
  onChange: (value: T) => void;
  items: TabItem<T>[];
};

export function Tabs<T extends TabValue>({ value, onChange, items }: TabsProps<T>) {
  return (
    <div className="tabs">
      <div className="tab-bar" role="tablist">
        {items.map((item) => (
          <button
            key={item.value}
            type="button"
            role="tab"
            aria-selected={value === item.value}
            className={`tab-button${value === item.value ? " active" : ""}`}
            onClick={() => onChange(item.value)}
          >
            <span className="tab-label">{item.label}</span>
            {item.badge !== undefined && <span className="tab-badge">{item.badge}</span>}
          </button>
        ))}
      </div>
      <div className="tab-panels">
        {items.map((item) => (
          <div
            key={item.value}
            role="tabpanel"
            hidden={value !== item.value}
            className="tab-panel"
          >
            {item.content}
          </div>
        ))}
      </div>
    </div>
  );
}