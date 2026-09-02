import React from "react";

export type HistoryDirection = "out" | "in";

export type HistoryEntry =
  | { id: string; direction: "out"; timestamp: number; text: string }
  | { id: string; direction: "out"; timestamp: number; names: string[] }
  | { id: string; direction: "in"; timestamp: number; text: string }
  | { id: string; direction: "in"; timestamp: number; name: string; path: string };

type HistoryPanelProps = {
  entries: HistoryEntry[];
  onClear?: () => void;
};

const TIME_FORMATTER = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

function formatTime(timestamp: number): string {
  return TIME_FORMATTER.format(new Date(timestamp));
}

function summaryFor(entry: HistoryEntry): React.ReactNode {
  if ("text" in entry) {
    const first = entry.text.split("\n", 1)[0] ?? "";
    if (entry.text.length > first.length || entry.text.includes("\n")) {
      return <><strong>{first || "(vacío)"}</strong> <span className="history-meta">+{Math.max(entry.text.length - first.length, 0)} más</span></>;
    }
    return <strong>{first || "(vacío)"}</strong>;
  }
  if ("names" in entry) {
    return <strong>{entry.names.length === 1 ? entry.names[0] : `${entry.names.length} archivos`}</strong>;
  }
  return <><strong>{entry.name}</strong><code className="history-path">{entry.path}</code></>;
}

function labelFor(entry: HistoryEntry): string {
  if ("text" in entry) return entry.direction === "out" ? "Enviaste texto" : "Recibiste texto";
  if ("names" in entry) return "Enviaste archivos";
  return "Recibiste archivo";
}

export function HistoryPanel({ entries, onClear }: HistoryPanelProps) {
  if (entries.length === 0) {
    return <p className="empty">Todavía no hay transferencias.</p>;
  }
  return <div className="history-panel">
    <div className="history-head">
      <p>Transferencias</p>
      <div className="history-head-actions">
        <span className="received-count">{entries.length}</span>
        {onClear && <button type="button" className="secondary" onClick={onClear}>Limpiar</button>}
      </div>
    </div>
    <ol className="history-list">
      {entries.map((entry) => (
        <li key={entry.id} className={`history-item history-${entry.direction}`}>
          <div className={`history-icon tint-${entry.direction}`}>
            <span>{entry.direction === "out" ? "↑" : "↓"}</span>
          </div>
          <div className="history-body">
            <div className="history-meta">
              <span className="history-label">{labelFor(entry)}</span>
              <span className="history-time">{formatTime(entry.timestamp)}</span>
            </div>
            <div className="history-content">{summaryFor(entry)}</div>
          </div>
        </li>
      ))}
    </ol>
  </div>;
}