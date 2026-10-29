// Window state and system settings the stylesheet reacts to through classes and properties on
// the root element.

import { isPreview } from "./preview";
import { getSystemAppearance } from "./tauri";

const INACTIVE_CLASS = "window-inactive";
const REDUCE_TRANSPARENCY_CLASS = "reduce-transparency";

/** Sets `window-inactive` on the root element while the window is not focused or not visible. */
export function trackWindowActive(root: HTMLElement = document.documentElement): () => void {
  const update = () => {
    const active = document.hasFocus() && document.visibilityState === "visible";
    root.classList.toggle(INACTIVE_CLASS, !active);
  };
  update();
  window.addEventListener("focus", update);
  window.addEventListener("blur", update);
  document.addEventListener("visibilitychange", update);
  return () => {
    window.removeEventListener("focus", update);
    window.removeEventListener("blur", update);
    document.removeEventListener("visibilitychange", update);
  };
}

/**
 * Applies the native accent colour as `--accent` and Reduce Transparency as a root class. Both
 * are read again whenever the window gains focus or the light or dark appearance changes.
 */
export function trackSystemAppearance(root: HTMLElement = document.documentElement): () => void {
  if (isPreview()) {
    return () => undefined;
  }
  const apply = () => {
    getSystemAppearance()
      .then((appearance) => {
        if (!appearance) {
          return;
        }
        if (appearance.accent) {
          root.style.setProperty("--accent", appearance.accent);
        } else {
          root.style.removeProperty("--accent");
        }
        root.classList.toggle(REDUCE_TRANSPARENCY_CLASS, appearance.reduce_transparency);
      })
      .catch((e) => console.error("could not read the system appearance", e));
  };
  apply();
  const scheme = window.matchMedia("(prefers-color-scheme: dark)");
  window.addEventListener("focus", apply);
  scheme.addEventListener("change", apply);
  return () => {
    window.removeEventListener("focus", apply);
    scheme.removeEventListener("change", apply);
  };
}
