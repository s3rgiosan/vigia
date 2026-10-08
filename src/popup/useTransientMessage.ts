import { useCallback, useEffect, useRef, useState } from "react";

/** A value that clears itself `ms` milliseconds after it is shown. */
export function useTransientMessage<T>(ms: number): [T | null, (next: T) => void, () => void] {
  const [value, setValue] = useState<T | null>(null);
  const timer = useRef<number | undefined>(undefined);

  const clear = useCallback(() => {
    window.clearTimeout(timer.current);
    setValue(null);
  }, []);

  const show = useCallback(
    (next: T) => {
      setValue(next);
      window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => setValue(null), ms);
    },
    [ms],
  );

  useEffect(() => () => window.clearTimeout(timer.current), []);

  return [value, show, clear];
}
