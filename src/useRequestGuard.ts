import { useCallback, useEffect, useRef } from "react";

// A ticket only remains valid for the latest request and current mounted inputs.
export function useRequestGuard(deps: unknown[] = []) {
  const generation = useRef(0);
  const active = useRef(false);
  useEffect(() => {
    active.current = true;
    return () => { active.current = false; generation.current++; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
  return useCallback(() => {
    const request = ++generation.current;
    return () => active.current && request === generation.current;
  }, []);
}
