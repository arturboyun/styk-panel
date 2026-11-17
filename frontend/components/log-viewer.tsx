"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowDownToLine, Download, Eraser, Pause, Play, Search, WrapText } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Dot } from "@/components/status";
import { wsUrl } from "@/lib/api";
import type { LogLine } from "@/lib/types";
import { cn } from "@/lib/utils";

const KEEP = 5000;
const LEVEL_RE = /\b(ERROR|ERR|FATAL|PANIC|CRITICAL|WARN(?:ING)?|INFO|DEBUG|TRACE)\b/i;

interface Line {
  n: number;
  s: LogLine["s"];
  ts: string;
  text: string;
  level: "error" | "warn" | null;
}

function parse(l: LogLine, n: number): Line {
  const raw = l.t.replace(/\n$/, "");
  const sp = raw.indexOf(" ");
  const hasTs = l.s !== "sys" && sp > 0 && /^\d{4}-\d\d-\d\dT/.test(raw);
  const text = hasTs ? raw.slice(sp + 1) : raw;
  const lvl = text.match(LEVEL_RE)?.[1]?.toUpperCase();
  return {
    n,
    s: l.s,
    ts: hasTs ? raw.slice(0, sp) : "",
    text,
    level: lvl && /ERR|FATAL|PANIC|CRIT/.test(lvl) ? "error" : lvl?.startsWith("WARN") ? "warn" : null,
  };
}

function Highlight({ text, needle }: { text: string; needle: string }) {
  if (!needle) return <>{text}</>;
  const parts = text.split(new RegExp(`(${needle.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "ig"));
  return (
    <>
      {parts.map((p, i) =>
        i % 2 ? (
          <mark key={i} className="rounded-sm bg-warning/40 text-inherit">
            {p}
          </mark>
        ) : (
          p
        ),
      )}
    </>
  );
}

export function LogViewer({ containerId, name }: { containerId: string; name: string }) {
  const [lines, setLines] = useState<Line[]>([]);
  const [tail, setTail] = useState("500");
  const [paused, setPaused] = useState(false);
  const [follow, setFollow] = useState(true);
  const [wrap, setWrap] = useState(true);
  const [filter, setFilter] = useState("");
  const [only, setOnly] = useState<"all" | "err" | "problems">("all");
  const [status, setStatus] = useState<"connecting" | "live" | "closed">("connecting");
  const counter = useRef(0);
  const buffer = useRef<Line[]>([]);
  const pausedRef = useRef(paused);
  useEffect(() => {
    pausedRef.current = paused;
  }, [paused]);
  const box = useRef<HTMLDivElement>(null);

  useEffect(() => {
    buffer.current = [];
    const ws = new WebSocket(wsUrl(`/docker/containers/${encodeURIComponent(containerId)}/logs?tail=${tail}&follow=true`));
    ws.onopen = () => {
      setLines([]);
      setStatus("live");
    };
    ws.onclose = () => setStatus("closed");
    ws.onmessage = (e) => {
      const batch = (JSON.parse(e.data) as LogLine[]).map((l) => parse(l, counter.current++));
      buffer.current.push(...batch);
    };
    // Flush on an interval so a chatty container does not re-render per line.
    const flush = setInterval(() => {
      if (pausedRef.current || buffer.current.length === 0) return;
      const add = buffer.current;
      buffer.current = [];
      setLines((prev) => {
        const next = prev.concat(add);
        return next.length > KEEP ? next.slice(next.length - KEEP) : next;
      });
    }, 250);
    return () => {
      clearInterval(flush);
      ws.close();
    };
  }, [containerId, tail]);

  const shown = useMemo(() => {
    const f = filter.toLowerCase();
    return lines.filter(
      (l) =>
        (only === "all" || (only === "err" ? l.s === "err" : l.level !== null)) && (!f || l.text.toLowerCase().includes(f)),
    );
  }, [lines, filter, only]);

  useEffect(() => {
    if (follow && box.current) box.current.scrollTop = box.current.scrollHeight;
  }, [shown, follow]);

  const onScroll = useCallback(() => {
    const el = box.current;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
    if (atBottom !== follow) setFollow(atBottom);
  }, [follow]);

  const download = () => {
    const blob = new Blob([lines.map((l) => `${l.ts} ${l.text}`).join("\n")], { type: "text/plain" });
    const a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = `${name}-${new Date().toISOString().slice(0, 19)}.log`;
    a.click();
    URL.revokeObjectURL(a.href);
  };

  const errors = lines.filter((l) => l.level === "error").length;

  return (
    <div className="flex h-[calc(100vh-17rem)] min-h-96 flex-col overflow-hidden rounded-xl border bg-card">
      <div className="flex flex-wrap items-center gap-2 border-b p-2">
        <div className="relative min-w-40 flex-1">
          <Search className="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Поиск в логах" className="pl-8" />
        </div>
        <Select value={only} onValueChange={(v) => setOnly(v as typeof only)} items={{ all: "Все строки", problems: "Ошибки и warn", err: "Только stderr" }}>
          <SelectTrigger className="w-40">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">Все строки</SelectItem>
            <SelectItem value="problems">Ошибки и warn</SelectItem>
            <SelectItem value="err">Только stderr</SelectItem>
          </SelectContent>
        </Select>
        <Select
          value={tail}
          items={Object.fromEntries(["100", "500", "2000", "5000"].map((t) => [t, `${t} строк`]))}
          onValueChange={(v) => {
            setStatus("connecting");
            setTail(String(v));
          }}
        >
          <SelectTrigger className="w-32">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {["100", "500", "2000", "5000"].map((t) => (
              <SelectItem key={t} value={t}>
                {t} строк
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <div className="flex items-center">
          <Tooltip>
            <TooltipTrigger render={<Button variant="ghost" size="icon" onClick={() => setPaused((p) => !p)} />}>
              {paused ? <Play /> : <Pause />}
            </TooltipTrigger>
            <TooltipContent>{paused ? "Продолжить" : "Пауза"}</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger render={<Button variant="ghost" size="icon" onClick={() => setWrap((w) => !w)} className={cn(wrap && "text-primary")} />}>
              <WrapText />
            </TooltipTrigger>
            <TooltipContent>Перенос строк</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger render={<Button variant="ghost" size="icon" onClick={() => setLines([])} />}>
              <Eraser />
            </TooltipTrigger>
            <TooltipContent>Очистить экран</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger render={<Button variant="ghost" size="icon" onClick={download} />}>
              <Download />
            </TooltipTrigger>
            <TooltipContent>Скачать</TooltipContent>
          </Tooltip>
        </div>
      </div>
      <div ref={box} onScroll={onScroll} className="relative flex-1 overflow-auto bg-background/40 py-2 font-mono text-[12.5px] leading-5">
        {shown.length === 0 && (
          <div className="p-6 text-center font-sans text-sm text-muted-foreground">
            {status === "connecting" ? "Подключение…" : filter || only !== "all" ? "Нет совпадений" : "Логов пока нет"}
          </div>
        )}
        {shown.map((l) => (
          <div
            key={l.n}
            className={cn(
              "flex gap-3 px-3 hover:bg-muted/50",
              l.level === "error" && "bg-destructive/5 text-destructive",
              l.level === "warn" && "text-warning",
              l.s === "sys" && "text-muted-foreground italic",
            )}
          >
            {l.ts && <span className="shrink-0 text-muted-foreground/60 select-none">{l.ts.slice(5, 19).replace("T", " ")}</span>}
            <span className={cn("min-w-0", wrap ? "break-all whitespace-pre-wrap" : "whitespace-pre")}>
              <Highlight text={l.text} needle={filter} />
            </span>
          </div>
        ))}
      </div>
      <div className="flex items-center gap-3 border-t px-3 py-1.5 text-xs text-muted-foreground">
        <span className="flex items-center gap-1.5">
          <Dot tone={status === "live" && !paused ? "ok" : status === "closed" ? "bad" : "warn"} pulse={status === "live" && !paused} />
          {paused ? "пауза" : status === "live" ? "в реальном времени" : status === "closed" ? "поток закрыт" : "подключение"}
        </span>
        <span className="tabular">
          {shown.length} / {lines.length} строк
        </span>
        {errors > 0 && <span className="text-destructive tabular">{errors} ошибок</span>}
        {!follow && (
          <button
            className="ml-auto flex items-center gap-1 text-primary hover:underline"
            onClick={() => {
              setFollow(true);
              box.current?.scrollTo({ top: box.current.scrollHeight });
            }}
          >
            <ArrowDownToLine className="size-3.5" /> к последним
          </button>
        )}
      </div>
    </div>
  );
}
