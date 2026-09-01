import React from "react";

export function TextPanel({ text }: { text: string }) {
  return <section className="clipboard-preview" aria-live="polite">
    <p>Último texto copiado</p>
    <pre>{text || "Aún no se ha copiado texto"}</pre>
  </section>;
}