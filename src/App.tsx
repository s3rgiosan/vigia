// Loaded first so component stylesheets, imported after it, win ties on specificity.
import "./App.css";
import { lazy, Suspense } from "react";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { viewFromSearch } from "./lib/view";

const Popup = lazy(() => import("./popup/Popup").then((m) => ({ default: m.Popup })));
const Settings = lazy(() => import("./settings/Settings").then((m) => ({ default: m.Settings })));
// The control gallery exists only on the dev server, for comparing against AppKit.
const Gallery = import.meta.env.DEV ? lazy(() => import("./dev/Gallery").then((m) => ({ default: m.Gallery }))) : null;

function App() {
  if (Gallery && new URLSearchParams(window.location.search).get("view") === "gallery") {
    return (
      <Suspense fallback={null}>
        <Gallery />
      </Suspense>
    );
  }

  const view = viewFromSearch(window.location.search);

  switch (view) {
    case "settings":
      return (
        <ErrorBoundary>
          <Suspense fallback={null}>
            <Settings />
          </Suspense>
        </ErrorBoundary>
      );
    case "popup":
    default:
      return (
        <ErrorBoundary>
          <Suspense fallback={null}>
            <Popup />
          </Suspense>
        </ErrorBoundary>
      );
  }
}

export default App;
