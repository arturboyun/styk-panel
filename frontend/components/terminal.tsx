"use client";

import { useEffect, useRef, useState } from "react";
import { Terminal as XTerm } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { RotateCw, SquareTerminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { wsUrl } from "@/lib/api";
import { Cancelled, useOps } from "@/lib/ops";

type Status = "idle" | "connecting" | "open" | "closed";

/** Interactive shell in a container (bash if present, else sh). Requires sudo. */
export function ContainerTerminal({ containerId, running }: { containerId: string; running: boolean }) {
  const host = useRef<HTMLDivElement>(null);
  const { ensureSudo } = useOps();
  const [status, setStatus] = useState<Status>("idle");
  const [session, setSession] = useState(0);

  useEffect(() => {
    if (!session || !host.current) return;
    const term = new XTerm({
      cursorBlink: true,
      fontFamily: "var(--font-mono), ui-monospace, monospace",
      fontSize: 13,
      theme: { background: "#0b0d10", foreground: "#d6dae0", cursor: "#d6dae0", selectionBackground: "#3b4252" },
      scrollback: 5000,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host.current);
    fit.fit();
    const ws = new WebSocket(wsUrl(`/docker/containers/${encodeURIComponent(containerId)}/exec?cols=${term.cols}&rows=${term.rows}`));
    ws.binaryType = "arraybuffer";
    setStatus("connecting");
    ws.onopen = () => {
      setStatus("open");
      term.focus();
    };
    ws.onmessage = (e) => {
      if (typeof e.data === "string") {
        term.write("\r\n\x1b[2m[сессия завершена]\x1b[0m\r\n");
        return;
      }
      term.write(new Uint8Array(e.data));
    };
    ws.onclose = () => setStatus("closed");
    const send = (m: object) => ws.readyState === WebSocket.OPEN && ws.send(JSON.stringify(m));
    const input = term.onData((d) => send({ t: "i", d }));
    const resize = term.onResize(({ cols, rows }) => send({ t: "r", c: cols, r: rows }));
    const ro = new ResizeObserver(() => fit.fit());
    ro.observe(host.current);
    return () => {
      ro.disconnect();
      input.dispose();
      resize.dispose();
      ws.close();
      term.dispose();
    };
  }, [session, containerId]);

  const connect = async () => {
    try {
      await ensureSudo();
      setSession((s) => s + 1);
    } catch (e) {
      if (!(e instanceof Cancelled)) throw e;
    }
  };

  if (!session)
    return (
      <div className="flex flex-col items-center gap-3 rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">
        <SquareTerminal className="size-8" />
        <p>Интерактивная оболочка внутри контейнера. Нужен режим sudo, открытие и закрытие сессии пишутся в аудит. Сессия закрывается после 15 минут без ввода.</p>
        <Button onClick={connect} disabled={!running}>
          {running ? "Подключиться" : "Контейнер не запущен"}
        </Button>
      </div>
    );

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between text-xs text-muted-foreground">
        <span>
          {status === "connecting" && "Подключение…"}
          {status === "open" && "Подключено"}
          {status === "closed" && "Соединение закрыто"}
        </span>
        {status === "closed" && (
          <Button size="sm" variant="outline" onClick={connect}>
            <RotateCw /> Переподключиться
          </Button>
        )}
      </div>
      <div className="h-[60vh] overflow-hidden rounded-xl border bg-[#0b0d10] p-2">
        <div ref={host} className="size-full" />
      </div>
    </div>
  );
}
