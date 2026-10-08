import { useCallback, useEffect, useState } from "react";
import { installUpdate, onUpdateProgress, subscribe, type UpdateProgress } from "../tauri";
import { useLatest } from "./useLatest";

/** Shown on the install button while nothing is installing. */
export const INSTALL_LABEL = "Install and Relaunch";

/** Download progress, or "installing" once every byte has arrived. Null while idle. */
type Phase = UpdateProgress | "installing" | null;

function labelFor(phase: Phase): string {
  if (phase === null) {
    return INSTALL_LABEL;
  }
  if (phase === "installing") {
    return "Installing…";
  }
  if (phase.total === null || phase.total <= 0) {
    return "Downloading…";
  }
  return `Downloading… ${Math.min(100, Math.floor((phase.downloaded / phase.total) * 100))}%`;
}

/**
 * Drives the install button: starts the install, follows the download progress and reports a
 * failure through `onError`. The app relaunches on success, so a finished install needs no state.
 */
export function useInstallUpdate(onError: (e: unknown) => void): { busy: boolean; label: string; install: () => void } {
  const [phase, setPhase] = useState<Phase>(null);
  const report = useLatest(onError);

  useEffect(
    () =>
      subscribe(
        onUpdateProgress((progress) => {
          setPhase((prev) => {
            if (prev === null) {
              return prev;
            }
            const done = progress.total !== null && progress.total > 0 && progress.downloaded >= progress.total;
            return done ? "installing" : progress;
          });
        }),
      ),
    [],
  );

  const install = useCallback(() => {
    setPhase({ downloaded: 0, total: null });
    installUpdate().catch((e) => {
      setPhase(null);
      report.current(e);
    });
  }, [report]);

  return { busy: phase !== null, label: labelFor(phase), install };
}
