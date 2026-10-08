/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Set by the Tauri CLI for `tauri dev` and `tauri build`: `darwin`, `linux` or `windows`. */
  readonly TAURI_ENV_PLATFORM?: string;
}
