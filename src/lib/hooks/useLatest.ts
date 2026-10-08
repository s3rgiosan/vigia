import { useLayoutEffect, useRef, type RefObject } from "react";

/** A ref that always holds the value from the latest committed render. */
export function useLatest<T>(value: T): RefObject<T> {
  const ref = useRef(value);
  useLayoutEffect(() => {
    ref.current = value;
  });
  return ref;
}
