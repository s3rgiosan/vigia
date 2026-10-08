// Symbols are SF Symbols exported by scripts/export-symbols.swift and drawn as a CSS mask, so they
// take the text color. The GitHub and GitLab brand marks stay as SVG.
import "../assets/symbols/symbols.css";

/** Every SF Symbol export, each drawn by the `.symbol--<name>` class in symbols.css. */
export const SYMBOL_NAMES = [
  "refresh",
  "pause",
  "play",
  "gear",
  "search",
  "disclosure",
  "person",
  "folder",
  "branch",
  "filter",
  "bell",
  "plus",
  "minus",
  "check",
  "warning",
  "external",
  "xmark",
] as const;

export type SymbolName = (typeof SYMBOL_NAMES)[number];
type BrandName = "github" | "gitlab";
export type IconName = SymbolName | BrandName;

const BRAND_PATHS: Record<BrandName, string> = {
  github:
    "M8 1.8a6.2 6.2 0 0 0-2 12.1c.3.05.4-.13.4-.3v-1.1c-1.7.37-2.1-.8-2.1-.8-.3-.7-.7-.9-.7-.9-.55-.38.04-.37.04-.37.6.04.95.63.95.63.55.94 1.45.67 1.8.5.05-.4.21-.67.39-.82-1.37-.16-2.8-.68-2.8-3.04 0-.67.24-1.22.63-1.65-.06-.16-.27-.78.06-1.63 0 0 .52-.17 1.7.63a5.8 5.8 0 0 1 3.1 0c1.18-.8 1.7-.63 1.7-.63.33.85.12 1.47.06 1.63.4.43.63.98.63 1.65 0 2.37-1.44 2.88-2.8 3.03.22.19.42.56.42 1.13v1.68c0 .17.1.35.42.29A6.2 6.2 0 0 0 8 1.8z",
  gitlab: "M8 13.8L2.2 9.5l1.3-6.3 1.6 4.7h5.8l1.6-4.7 1.3 6.3z",
};

function isBrand(name: IconName): name is BrandName {
  return name === "github" || name === "gitlab";
}

export function Icon({ name, size = 16, className }: { name: IconName; size?: number; className?: string }) {
  if (isBrand(name)) {
    return (
      <svg
        className={className}
        width={size}
        height={size}
        viewBox="0 0 16 16"
        aria-hidden="true"
        focusable="false"
        fill="currentColor"
      >
        <path d={BRAND_PATHS[name]} />
      </svg>
    );
  }

  const classes = className ? `symbol symbol--${name} ${className}` : `symbol symbol--${name}`;
  return <span className={classes} aria-hidden="true" style={{ width: size, height: size }} />;
}
