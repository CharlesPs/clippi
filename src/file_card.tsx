import React from "react";

export type ClipboardFile = { path: string; name: string; size: number; mime: string };

type FileCardProps = { files: ClipboardFile[] };

function formatSize(bytes: number): string {
  if (bytes <= 0) { return "0 B"; }
  const units = ["B", "KB", "MB", "GB", "TB"];
  const power = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const value = bytes / Math.pow(1024, power);
  const formatted = power === 0 ? value.toFixed(0) : value.toFixed(value >= 100 ? 0 : 1);
  return `${formatted} ${units[power]}`;
}

function iconFor(mime: string, name: string, isLink: boolean): { symbol: string; tint: string } {
  if (isLink) return { symbol: "🔗", tint: "tint-sky" };
  if (mime.startsWith("image/")) return { symbol: "🖼", tint: "tint-violet" };
  if (mime.startsWith("video/")) return { symbol: "🎬", tint: "tint-rose" };
  if (mime.startsWith("audio/")) return { symbol: "🎵", tint: "tint-amber" };
  if (mime === "application/pdf") return { symbol: "📕", tint: "tint-rose" };
  if (mime === "application/zip" || mime.includes("compressed") || mime.includes("gzip") || mime.includes("tar")) return { symbol: "📦", tint: "tint-amber" };
  if (mime.startsWith("text/x-c") || mime.startsWith("text/x-rust") || mime.startsWith("text/x-python") || mime.startsWith("text/x-go") || mime.startsWith("text/x-java") || mime.startsWith("text/x-ruby") || mime === "application/javascript" || mime === "application/typescript" || mime === "application/json" || mime === "text/x-shellscript" || name.endsWith(".rs") || name.endsWith(".py") || name.endsWith(".js") || name.endsWith(".ts") || name.endsWith(".json") || name.endsWith(".sh")) return { symbol: "⌨", tint: "tint-emerald" };
  if (mime.startsWith("text/")) return { symbol: "📄", tint: "tint-sky" };
  if (mime.includes("officedocument") || mime.includes("msword") || mime.includes("ms-excel") || mime.includes("ms-powerpoint")) return { symbol: "📊", tint: "tint-emerald" };
  return { symbol: "📎", tint: "tint-sky" };
}

function isHttp(path: string): boolean {
  return path.startsWith("http://") || path.startsWith("https://");
}

export function FileCard({ files }: FileCardProps) {
  if (files.length === 0) return null;
  return <section className="file-card" aria-live="polite">
    <div className="file-card-head">
      <div className="file-card-title">
        <span className="file-card-eyebrow">Portapapeles</span>
        <h3>{files.length === 1 ? "1 archivo" : `${files.length} archivos`}</h3>
      </div>
      <span className="file-card-hint">Ctrl+V donde quieras para pegar</span>
    </div>
    <ul className="file-card-list">
      {files.map((file, index) => {
        const { symbol, tint } = iconFor(file.mime, file.name, isHttp(file.path));
        const pathClass = isHttp(file.path) ? "file-card-path file-card-path-link" : "file-card-path";
        return <li key={`${file.path}-${index}`} className="file-card-item">
          <div className={`file-card-icon ${tint}`}><span>{symbol}</span></div>
          <div className="file-card-info">
            <div className="file-card-name" title={file.name}>{file.name}</div>
            <div className="file-card-meta">
              <span className="file-card-size">{formatSize(file.size)}</span>
              <span className="file-card-divider" />
              <code className={pathClass} title={file.path}>{file.path}</code>
            </div>
          </div>
        </li>;
      })}
    </ul>
  </section>;
}