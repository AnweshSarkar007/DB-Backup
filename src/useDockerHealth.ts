import { useCallback, useEffect, useRef, useState } from "react";
import { DockerHealth, ErrorReport, dockerHealth } from "./api";
import { normalizeError } from "./errors";

export function useDockerHealth() {
  const [data, setData] = useState<DockerHealth | null>(null);
  const [error, setError] = useState<ErrorReport | null>(null);
  const [loading, setLoading] = useState(true);
  const active = useRef(false);
  const checking = useRef(false);
  const recheck = useCallback(async () => {
    if (!active.current || checking.current) return;
    checking.current = true;
    setLoading(true);
    setError(null);
    try {
      const health = await dockerHealth();
      if (active.current) setData(health);
    } catch (error) {
      if (active.current) setError(normalizeError(error));
    } finally {
      checking.current = false;
      if (active.current) setLoading(false);
    }
  }, []);
  useEffect(() => {
    active.current = true;
    void recheck();
    const focus = () => { void recheck(); };
    window.addEventListener("focus", focus);
    return () => { active.current = false; window.removeEventListener("focus", focus); };
  }, [recheck]);
  return { data, error, loading, recheck };
}
