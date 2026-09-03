import React, { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

type FileReceived = { id: string; name: string; path: string };

type MenuState = { x: number; y: number; path: string; name: string } | null;

export function ReceivedPanel() {
  const [received, setReceived] = useState<FileReceived[]>([]);
  const [menu, setMenu] = useState<MenuState>(null);
  const [feedback, setFeedback] = useState<string | null>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const feedbackTimer = useRef<number | null>(null);

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

  useEffect(() => {
    if (!menu) return;
    function onPointer(event: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenu(null);
      }
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") setMenu(null);
    }
    window.addEventListener("mousedown", onPointer);
    window.addEventListener("keydown", onKey);
    window.addEventListener("contextmenu", onPointer);
    return () => {
      window.removeEventListener("mousedown", onPointer);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("contextmenu", onPointer);
    };
  }, [menu]);

  useEffect(() => () => {
    if (feedbackTimer.current !== null) window.clearTimeout(feedbackTimer.current);
  }, []);

  function flash(message: string) {
    setFeedback(message);
    if (feedbackTimer.current !== null) window.clearTimeout(feedbackTimer.current);
    feedbackTimer.current = window.setTimeout(() => setFeedback(null), 2200);
  }

  function openMenu(event: React.MouseEvent, file: FileReceived) {
    event.preventDefault();
    setMenu({ x: event.clientX, y: event.clientY, path: file.path, name: file.name });
  }

  async function pasteItem() {
    const target = menu;
    setMenu(null);
    if (!target) return;
    try {
      await invoke("paste_received_file", { path: target.path });
      flash(`"${target.name}" listo para pegar`);
    } catch (error) {
      flash(`No se pudo publicar: ${error}`);
    }
  }

  if (received.length === 0) return null;

  const adjusted = menu ? positionMenu(menu, menuRef.current) : null;

  return <section className="received">
    <div className="received-head">
      <p>Recibidos del otro equipo</p>
      <span className="received-count">{received.length}</span>
    </div>
    <ul className="received-list">
      {received.map((file) => <li key={file.id} className="received-item" onContextMenu={(event) => openMenu(event, file)}>
        <span className="received-name">{file.name}</span>
        <code className="received-path">{file.path}</code>
      </li>)}
    </ul>
    {feedback && <div className="received-toast" role="status" aria-live="polite">{feedback}</div>}
    {menu && adjusted && <div ref={menuRef} className="context-menu" style={{ left: adjusted.x, top: adjusted.y }} role="menu">
      <button type="button" className="context-menu-item" role="menuitem" onClick={() => void pasteItem()}>
        <span className="context-menu-icon" aria-hidden="true">📋</span>
        <span>Paste Item</span>
      </button>
    </div>}
  </section>;
}

function positionMenu(menu: { x: number; y: number }, node: HTMLDivElement | null) {
  const padding = 8;
  const width = node?.offsetWidth ?? 180;
  const height = node?.offsetHeight ?? 40;
  const maxX = window.innerWidth - width - padding;
  const maxY = window.innerHeight - height - padding;
  return {
    x: Math.max(padding, Math.min(menu.x, maxX)),
    y: Math.max(padding, Math.min(menu.y, maxY)),
  };
}
