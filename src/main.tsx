import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { TextPanel } from "./texts";
import { ReceivedPanel } from "./received";
import { FileCard, ClipboardFile } from "./file_card";
import { Tabs } from "./tabs";
import { HistoryPanel, HistoryEntry } from "./history";
import "./styles.scss";

type Event = { state: "connected" | "disconnected" | "server_started" | "sent" | "received" | "error"; detail: string };
type ClipboardUpdate = { text: string };
type ClipboardFilesEvent = { files: ClipboardFile[] };
type FileReceived = { id: string; name: string; path: string };
type FileFinalized = { id: string; name: string; direction: "send" | "recv" };
type ClipboardMode = "none" | "text" | "files";
type TabId = "connection" | "clipboard" | "history";
type ActivityKind = "info" | "success" | "warning" | "error";

const defaultEndpoint = "ws://127.0.0.1:8787";

function entryId() { return crypto.randomUUID(); }

function kindForState(state: Event["state"]): ActivityKind {
  if (state === "connected" || state === "server_started") return "success";
  if (state === "disconnected") return "warning";
  if (state === "error") return "error";
  return "info";
}

function App() {
  const [tab, setTab] = useState<TabId>("connection");
  const [endpoint, setEndpoint] = useState(localStorage.getItem("endpoint") ?? defaultEndpoint);
  const [room, setRoom] = useState(localStorage.getItem("room") ?? "mi-sala");
  const [role, setRole] = useState<"client" | "server">((localStorage.getItem("role") as "client" | "server") ?? "client");
  const [port, setPort] = useState(localStorage.getItem("relayPort") ?? "8787");
  const [connected, setConnected] = useState(false);
  const [activity, setActivity] = useState("Sin conexión");
  const [activityKind, setActivityKind] = useState<ActivityKind>("info");
  const [clipboardMode, setClipboardMode] = useState<ClipboardMode>("none");
  const [clipboardText, setClipboardText] = useState("");
  const [clipboardFiles, setClipboardFiles] = useState<ClipboardFile[]>([]);
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  const clientId = useMemo(() => {
    const old = localStorage.getItem("clientId");
    if (old) return old;
    const next = crypto.randomUUID(); localStorage.setItem("clientId", next); return next;
  }, []);

  useEffect(() => {
    let active = true;
    const unlistenStatus = listen<Event>("sync-status", ({ payload }) => {
      if (!active) return;
      setActivity(payload.detail);
      setActivityKind(kindForState(payload.state));
      if (payload.state === "connected" || payload.state === "server_started") {
        setConnected(true);
        setTab("clipboard");
      } else if (payload.state === "disconnected" || payload.state === "error") {
        setConnected(false);
        setTab("connection");
        setClipboardMode("none");
        setClipboardText("");
        setClipboardFiles([]);
        setHistory([]);
      }
    });
    const unlistenText = listen<ClipboardUpdate>("clipboard-update", ({ payload }) => {
      if (!active) return;
      setClipboardText(payload.text);
      setClipboardMode("text");
      setTab("clipboard");
    });
    const unlistenFiles = listen<ClipboardFilesEvent>("clipboard-files", ({ payload }) => {
      if (!active) return;
      setClipboardFiles(payload.files);
      setClipboardMode(payload.files.length > 0 ? "files" : "none");
      setTab("clipboard");
    });
    const unlistenTextSent = listen<ClipboardUpdate>("text-sent", ({ payload }) => {
      if (!active) return;
      const entry: HistoryEntry = { id: entryId(), direction: "out", timestamp: Date.now(), text: payload.text };
      setHistory((es) => [entry, ...es].slice(0, 200));
    });
    const unlistenTextReceived = listen<ClipboardUpdate>("text-received", ({ payload }) => {
      if (!active) return;
      const entry: HistoryEntry = { id: entryId(), direction: "in", timestamp: Date.now(), text: payload.text };
      setHistory((es) => [entry, ...es].slice(0, 200));
    });
    const unlistenFileSent = listen<FileFinalized>("file-sent", ({ payload }) => {
      if (!active) return;
      const entry: HistoryEntry = { id: entryId(), direction: "out", timestamp: Date.now(), names: [payload.name] };
      setHistory((es) => [entry, ...es].slice(0, 200));
    });
    const unlistenFileReceived = listen<FileReceived>("file-received", ({ payload }) => {
      if (!active) return;
      const entry: HistoryEntry = { id: entryId(), direction: "in", timestamp: Date.now(), name: payload.name, path: payload.path };
      setHistory((es) => [entry, ...es].slice(0, 200));
    });
    return () => {
      active = false;
      void Promise.all([unlistenStatus, unlistenText, unlistenFiles, unlistenTextSent, unlistenTextReceived, unlistenFileSent, unlistenFileReceived]).then((listeners) => listeners.forEach((fn) => fn()));
    };
  }, []);

  async function connect() {
    localStorage.setItem("endpoint", endpoint); localStorage.setItem("room", room); localStorage.setItem("role", role); localStorage.setItem("relayPort", port);
    await invoke("connect", { request: { endpoint, room, clientId } });
  }
  async function startRelay() {
    localStorage.setItem("role", role); localStorage.setItem("relayPort", port);
    await invoke("start_relay", { request: { port: Number(port), room, clientId } });
  }
  async function disconnect() { await invoke("disconnect"); }

  const portapapelesContent = (() => {
    if (clipboardMode === "files") return <FileCard files={clipboardFiles} />;
    if (clipboardMode === "text") return <TextPanel text={clipboardText} />;
    return <p className="empty">Copiá texto o un archivo (Ctrl+C) para verlo acá.</p>;
  })();

  return <main>
    <section className="card">
      <header className="card-header">
        <div className="card-titles">
          <p className="eyebrow">MVP · texto y archivos · red local</p>
          <h1>Clipboard Sync</h1>
        </div>
        <span className={`status-pill ${connected ? "online" : "offline"}`}>
          <span className="status-dot" />
          {connected ? (role === "server" ? "Relay activo" : "Conectado") : "Desconectado"}
        </span>
      </header>
      <Tabs<TabId>
        value={tab}
        onChange={setTab}
        items={[
          {
            value: "connection",
            label: "Conexión",
            content: <section className="panel panel-connection">
              <div className="role-picker" role="radiogroup" aria-label="Rol">
                <button type="button" className={role === "client" ? "selected" : "secondary"} onClick={() => setRole("client")} disabled={connected}>Cliente</button>
                <button type="button" className={role === "server" ? "selected" : "secondary"} onClick={() => setRole("server")} disabled={connected}>Servidor</button>
              </div>
              {role === "client" ? <>
                <label>Dirección del relay<input value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="ws://192.168.1.20:8787" /></label>
                <label>Sala compartida<input value={room} onChange={(e) => setRoom(e.target.value)} placeholder="mi-sala" /></label>
                <p className="hint">Usa la misma dirección y sala en ambos clientes. No uses datos sensibles en este MVP.</p>
                <div className="actions"><button type="button" onClick={() => void connect()} disabled={connected}>Conectar</button><button type="button" className="secondary" onClick={() => void disconnect()} disabled={!connected}>Desconectar</button></div>
              </> : <>
                <label>Puerto del relay<input inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value)} /></label>
                <label>Sala compartida<input value={room} onChange={(e) => setRoom(e.target.value)} placeholder="mi-sala" /></label>
                <p className="hint">Este equipo también sincroniza su portapapeles. En los clientes usa <code>ws://IP-DE-ESTE-EQUIPO:{port}</code> y la misma sala.</p>
                <div className="actions"><button type="button" onClick={() => void startRelay()} disabled={connected}>Iniciar relay</button><button type="button" className="secondary" onClick={() => void disconnect()} disabled={!connected}>Detener</button></div>
              </>}
            </section>,
          },
          {
            value: "clipboard",
            label: "Portapapeles",
            badge: clipboardMode !== "none" ? <span className="tab-badge-dot" aria-label="contenido en portapapeles" /> : undefined,
            content: <section className="panel panel-clipboard">
              <p className="panel-section-title">Ahora en tu portapapeles <span className="panel-section-mode">{clipboardMode === "none" ? "vacío" : clipboardMode === "text" ? "texto" : "archivos"}</span></p>
              {portapapelesContent}
            </section>,
          },
          {
            value: "history",
            label: "Historial",
            badge: history.length > 0 ? history.length : undefined,
            content: <section className="panel panel-history">
              <div className="panel-section"><ReceivedPanel /></div>
              <div className="panel-section"><HistoryPanel entries={history} onClear={() => setHistory([])} /></div>
            </section>,
          },
        ]}
      />
    </section>
    <footer className={`activity activity-${activityKind}`} role="status" aria-live="polite">
      <span className="activity-dot" />
      <span className="activity-text">{activity}</span>
    </footer>
  </main>;
}
createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);