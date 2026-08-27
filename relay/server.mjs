import { WebSocketServer, WebSocket } from "ws";

const port = Number(process.env.PORT ?? 8787);
const wss = new WebSocketServer({ host: "0.0.0.0", port });

wss.on("connection", (socket) => {
  socket.on("message", (raw) => {
    let message;
    try { message = JSON.parse(raw.toString()); } catch { return; }
    if (message.type === "join" && typeof message.room === "string") { socket.room = message.room; return; }
    if (message.type !== "clipboard" || typeof message.room !== "string" || typeof message.text !== "string") return;
    if (message.text.length > 1_000_000) return;
    socket.room = message.room;
    const outbound = JSON.stringify(message);
    for (const peer of wss.clients) {
      if (peer !== socket && peer.room === message.room && peer.readyState === WebSocket.OPEN) peer.send(outbound);
    }
  });
});

console.log(`Clipboard relay available on ws://0.0.0.0:${port}`);
