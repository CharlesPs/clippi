import React, { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

type FileReceived = { id: string; name: string; path: string };
type FileFailed = { id: string; name: string | null; direction: "send" | "recv"; error: string };

export function FilesPanel() {
  const [received, setReceived] = useState<FileReceived[]>([]);
  const [failures, setFailures] = useState<FileFailed[]>([]);

  useEffect(() => {
    let active = true;
    const unlistenReceived = listen<FileReceived>("file-received", ({ payload }) => {
      if (!active) return;
      setReceived((list) => [payload, ...list].slice(0, 20));
    });
    const unlistenFailed = listen<FileFailed>("file-failed", ({ payload }) => {
      if (!active) return;
      setFailures((list) => [payload, ...list].slice(0, 5));
    });
    return () => {
      active = false;
      void Promise.all([unlistenReceived, unlistenFailed]).then((listeners) => listeners.forEach((fn) => fn()));
    };
  }, []);

  if (received.length === 0 && failures.length === 0) return null;

  return <section className="files-panel">
    {failures.length > 0 && <ul className="failure-list">
      {failures.map((failure, index) => <li key={`${failure.id}-${index}`} className="failure"><span className="failure-dot" />No se pudo enviar <strong>{failure.name ?? "archivo"}</strong>: {failure.error}</li>)}
    </ul>}
    {received.length > 0 && <section className="received"><p>Últimos recibidos</p><ul>{received.map((file) => <li key={file.id}><span className="received-name">{file.name}</span><code className="received-path">{file.path}</code></li>)}</ul></section>}
  </section>;
}