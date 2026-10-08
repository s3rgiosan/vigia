/** True when the app runs in a plain browser against the fake backend. */
export function isPreview(): boolean {
  return "__VIGIA_PREVIEW__" in window;
}
