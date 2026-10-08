import { useCallback, useEffect, useRef } from "react";
import { friendlyError } from "../lib/errors";
import { updateSettings, type Settings } from "../lib/tauri";

/**
 * Saves settings patches one after another. Each patch applies to the result of the previous
 * one, so quick consecutive changes never overwrite each other with stale values.
 */
export function useSettingsSaver(
  settings: Settings | null,
  onChange: () => Promise<void>,
  onError: (message: string | null) => void,
): (patch: Partial<Settings>) => Promise<void> {
  const latest = useRef<Settings | null>(settings);
  const queued = useRef(0);
  const chain = useRef<Promise<void>>(Promise.resolve());

  // Adopt changes made elsewhere while no save is in flight.
  useEffect(() => {
    if (queued.current === 0 && settings !== null) {
      latest.current = settings;
    }
  }, [settings]);

  return useCallback(
    (patch: Partial<Settings>) => {
      queued.current += 1;
      const job = chain.current.then(async () => {
        const previous = latest.current;
        if (previous === null) {
          onError("Settings are not loaded yet.");
          return;
        }
        latest.current = { ...previous, ...patch };
        try {
          await updateSettings(latest.current);
          onError(null);
          await onChange();
        } catch (e) {
          latest.current = previous;
          onError(friendlyError(e));
        }
      });
      chain.current = job.finally(() => {
        queued.current -= 1;
      });
      return job;
    },
    [onChange, onError],
  );
}
