import { WebSocketServer, WebSocket } from "ws";

const port = Number(process.env.PORT ?? 8787);
const MAX_TEXT_BYTES = 1_000_000;
const wss = new WebSocketServer({ host: "0.0.0.0", port });

wss.on("connection", (socket) => {
  socket.on("message", (raw, isBinary) => {
    if (isBinary) {
      const outbound = raw;
      for (const peer of wss.clients) {
        if (peer !== socket && peer.readyState === WebSocket.OPEN) peer.send(outbound, { binary: true });
      }
      return;
    }
    let message;
    try { message = JSON.parse(raw.toString()); } catch { return; }
    if (message.type === "join" && typeof message.room === "string") {
      socket.room = message.room;
      const outbound = JSON.stringify(message);
      for (const peer of wss.clients) {
        if (peer !== socket && peer.room === message.room && peer.readyState === WebSocket.OPEN) peer.send(outbound);
      }
      return;
    }
    if (typeof message.room !== "string") return;
    socket.room = message.room;
    if (message.type === "clipboard") {
      if (typeof message.text !== "string" || message.text.length > MAX_TEXT_BYTES) return;
      const outbound = JSON.stringify(message);
      for (const peer of wss.clients) {
        if (peer !== socket && peer.room === message.room && peer.readyState === WebSocket.OPEN) peer.send(outbound);
      }
      return;
    }
    if (message.type === "file_start" || message.type === "file_done" || message.type === "file_cancel") {
      // Legacy envelope kinds from the old binary chunk transfer flow.
      // The new lazy-pull flow doesn't use them; ignore if received.
      return;
    }
  });
});

console.log(`Clipboard relay available on ws://0.0.0.0:${port}`);