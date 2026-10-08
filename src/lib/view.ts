export type View = "popup" | "settings";

/** Picks the window view from the `view` query parameter. Unknown values fall back to the popup. */
export function viewFromSearch(search: string): View {
  const value = new URLSearchParams(search).get("view");
  switch (value) {
    case "settings":
      return "settings";
    default:
      return "popup";
  }
}
