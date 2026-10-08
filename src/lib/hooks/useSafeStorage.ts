import { useCallback, useState } from "react";

function read(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

/** A string kept in localStorage. Storage that is missing or throws falls back to memory. */
export function useSafeStorage(key: string, fallback: string): [string, (value: string) => void] {
  const [value, setValue] = useState(() => read(key, fallback));

  const update = useCallback(
    (next: string) => {
      setValue(next);
      try {
        localStorage.setItem(key, next);
      } catch {
        // The value stays in memory for this session.
      }
    },
    [key],
  );

  return [value, update];
}
