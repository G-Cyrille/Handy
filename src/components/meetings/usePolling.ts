import { useCallback, useEffect, useRef, useState } from "react";
import { errorMessage } from "./meetingFormat";

interface PollingState<T> {
  data: T | null;
  error: string | null;
  refresh: () => Promise<void>;
}

/**
 * Calls `load` immediately, then again `intervalMs` after each call settles,
 * while the component is mounted and `enabled` is true. Calls never overlap.
 * The backend keeps capturing whether or not anything polls it.
 */
export const usePolling = <T>(
  load: () => Promise<T>,
  intervalMs: number,
  enabled = true,
): PollingState<T> => {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mountedRef = useRef(true);
  const loadRef = useRef(load);
  loadRef.current = load;

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    try {
      const next = await loadRef.current();
      if (!mountedRef.current) return;
      setData(next);
      setError(null);
    } catch (e) {
      if (mountedRef.current) setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      await refresh();
      if (!cancelled) timer = setTimeout(tick, intervalMs);
    };
    tick();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [enabled, intervalMs, refresh]);

  return { data, error, refresh };
};
