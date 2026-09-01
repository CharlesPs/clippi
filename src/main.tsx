import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { TextPanel } from "./texts";
import { FilesPanel } from "./files";
import { FileCard, ClipboardFile } from "./file_card";
import "./styles.scss";

type Event = { state: "connected" | "disconnected" | "server_started" | "sent" | "received" | "error"; detail: string };
type ClipboardUpdate = { text: string };
type ClipboardFilesEvent = { files: ClipboardFile[] };
type ClipboardKind = "text" | "files";

const defaultEndpoint = "ws://127.0.0.1:8787";

function App() {
  const [endpoint, setEndpoint] = useState(localStorage.getItem("endpoint") ?? defaultEndpoint);
  const [room, setRoom] = useState(localStorage.getItem("room") ?? "mi-sala");
  const [role, setRole] = useState<"client" | "server">((localStorage.getItem("role") as "client" | "server") ?? "client");
  const [port, setPort] = useState(localStorage.getItem("relayPort") ?? "8787");
  const [connected, setConnected] = useState(false);
  const [activity, setActivity] = useState("Aún no conectado");
  const [clipboardText, setClipboardText] = useState("");
  const [clipboardFiles, setClipboardFiles] = useState<ClipboardFile[]>([]);
  const [clipboardKind, setClipboardKind] = useState<ClipboardKind>("text");
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
      if (payload.state === "connected" || payload.state === "server_started") setConnected(true);
      if (payload.state === "disconnected" || payload.state === "error") setConnected(false);
    });
    const unlistenText = listen<ClipboardUpdate>("clipboard-update", ({ payload }) => {
      if (!active) return;
      setClipboardText(payload.text);
      setClipboardKind("text");
    });
    const unlistenFiles = listen<ClipboardFilesEvent>("clipboard-files", ({ payload }) => {
      if (!active) return;
      setClipboardFiles(payload.files);
      setClipboardKind("files");
    });
    return () => {
      active = false;
      void Promise.all([unlistenStatus, unlistenText, unlistenFiles]).then((listeners) => listeners.forEach((fn) => fn()));
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

  return <main>
    <section className="card"><p className="eyebrow">MVP · texto y archivos · red local</p><h1>Clipboard Sync</h1>
      <p className={connected ? "status online" : "status"}>{connected ? (role === "server" ? "● Relay activo" : "● Conectado") : "○ Desconectado"}</p>
      <div className="role-picker"><button className={role === "client" ? "selected" : "secondary"} onClick={() => setRole("client")} disabled={connected}>Cliente</button><button className={role === "server" ? "selected" : "secondary"} onClick={() => setRole("server")} disabled={connected}>Servidor</button></div>
      {role === "client" ? <>
        <label>Dirección del relay<input value={endpoint} onChange={(e) => setEndpoint(e.target.value)} placeholder="ws://192.168.1.20:8787" /></label>
        <label>Sala compartida<input value={room} onChange={(e) => setRoom(e.target.value)} placeholder="mi-sala" /></label>
        <p className="hint">Usa la misma dirección y sala en ambos clientes. No uses datos sensibles en este MVP.</p>
        <div className="actions"><button onClick={() => void connect()} disabled={connected}>Conectar</button><button className="secondary" onClick={() => void disconnect()} disabled={!connected}>Desconectar</button></div>
      </> : <>
        <label>Puerto del relay<input inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value)} /></label>
        <label>Sala compartida<input value={room} onChange={(e) => setRoom(e.target.value)} placeholder="mi-sala" /></label>
        <p className="hint">Este equipo también sincroniza su portapapeles. En los clientes usa <code>ws://IP-DE-ESTE-EQUIPO:{port}</code> y la misma sala.</p>
        <div className="actions"><button onClick={() => void startRelay()} disabled={connected}>Iniciar relay</button><button className="secondary" onClick={() => void disconnect()} disabled={!connected}>Detener</button></div>
      </>}
      <div className="activity">{activity}</div>
      {clipboardKind === "files" && clipboardFiles.length > 0
        ? <FileCard files={clipboardFiles} />
        : <TextPanel text={clipboardText} />}
      <FilesPanel />
    </section>
  </main>;
}
createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);