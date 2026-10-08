import { useEffect, useState } from "react";

function isActive(): boolean {
  return document.visibilityState !== "hidden" && document.hasFocus();
}

/** True while the document is visible and its window has focus. */
export function useDocumentVisible(): boolean {
  const [active, setActive] = useState(isActive);

  useEffect(() => {
    const update = () => setActive(isActive());
    document.addEventListener("visibilitychange", update);
    window.addEventListener("focus", update);
    window.addEventListener("blur", update);
    update();
    return () => {
      document.removeEventListener("visibilitychange", update);
      window.removeEventListener("focus", update);
      window.removeEventListener("blur", update);
    };
  }, []);

  return active;
}
