"use client";

import { createContext, useContext, useEffect, useRef, useState } from "react";
import { wsUrl } from "@/lib/api";
import type { Sample } from "@/lib/types";

const MAX = 720;

interface Live {
  samples: Sample[];
  latest: Sample | null;
  connected: boolean;
}

const LiveContext = createContext<Live>({ samples: [], latest: null, connected: false });

/** One WebSocket per tab feeding every live widget; reconnects with backoff. */
export function LiveProvider({ children }: { children: React.ReactNode }) {
  const [state, setState] = useState<Live>({ samples: [], latest: null, connected: false });
  const retry = useRef(0);

  useEffect(() => {
    let ws: WebSocket | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let closed = false;

    const connect = () => {
      ws = new WebSocket(wsUrl("/system/live"));
      ws.onopen = () => {
        retry.current = 0;
        setState((s) => ({ ...s, connected: true }));
      };
      ws.onmessage = (e) => {
        const sample = JSON.parse(e.data) as Sample;
        setState((s) => {
          const samples = s.samples.length >= MAX ? [...s.samples.slice(1), sample] : [...s.samples, sample];
          return { samples, latest: sample, connected: true };
        });
      };
      ws.onclose = () => {
        setState((s) => ({ ...s, connected: false }));
        if (closed) return;
        const delay = Math.min(30_000, 1000 * 2 ** retry.current++);
        timer = setTimeout(connect, delay);
      };
    };
    connect();
    return () => {
      closed = true;
      clearTimeout(timer);
      ws?.close();
    };
  }, []);

  return <LiveContext.Provider value={state}>{children}</LiveContext.Provider>;
}

export const useLive = () => useContext(LiveContext);

/** Live per-container numbers keyed by container name. */
export function useContainerStats() {
  const { latest } = useLive();
  const map = new Map<string, Sample["containers"][number]>();
  latest?.containers.forEach((c) => map.set(c.name, c));
  return map;
}
