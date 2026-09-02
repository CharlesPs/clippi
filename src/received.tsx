import React, { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";

type FileReceived = { id: string; name: string; path: string };

export function ReceivedPanel() {
  const [received, setReceived] = useState<FileReceived[]>([]);

  useEffect(() => {
    let active = true;
    const unlistenReceived = listen<FileReceived>("file-received", ({ payload }) => {
      if (!active) return;
      setReceived((list) => [payload, ...list].slice(0, 20));
    });
    return () => {
      active = false;
      void unlistenReceived.then((fn) => fn());
    };
  }, []);

  if (received.length === 0) return null;

  return <section className="received">
    <div className="received-head">
      <p>Recibidos del otro equipo</p>
      <span className="received-count">{received.length}</span>
    </div>
    <ul className="received-list">
      {received.map((file) => <li key={file.id} className="received-item">
        <span className="received-name">{file.name}</span>
        <code className="received-path">{file.path}</code>
      </li>)}
    </ul>
  </section>;
}