import React from "react";
import ReactDOM from "react-dom/client";

async function start() {
  // A plain browser during `npm run dev` gets a fake backend for design previews.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { install } = await import("./dev/mock");
    install();
  }
  // App is loaded after the fake backend so its modules see the preview flag.
  const { default: App } = await import("./App");

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

start();
