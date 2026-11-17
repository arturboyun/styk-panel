"use client";

import { useEffect, useRef, useState } from "react";
import { CheckCircle2, Loader2, XCircle } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { api } from "@/lib/api";
import { dateTime } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { JobTail } from "@/lib/types";

export function JobStatus({ status }: { status: string }) {
  if (status === "running")
    return (
      <Badge variant="outline" className="gap-1">
        <Loader2 className="size-3 animate-spin" /> выполняется
      </Badge>
    );
  if (status === "ok")
    return (
      <Badge variant="outline" className="gap-1 border-success/40 text-success">
        <CheckCircle2 className="size-3" /> готово
      </Badge>
    );
  return (
    <Badge variant="destructive" className="gap-1">
      <XCircle className="size-3" /> {status === "lost" ? "потеряна" : "ошибка"}
    </Badge>
  );
}

/** Live output of a job; polls until it finishes. */
export function JobLog({ id, onStatus, className }: { id: number; onStatus?: (s: string) => void; className?: string }) {
  const [lines, setLines] = useState<string[]>([]);
  const [job, setJob] = useState<JobTail | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const stick = useRef(true);
  const report = useRef(onStatus);
  useEffect(() => {
    report.current = onStatus;
  });

  useEffect(() => {
    let offset = 0;
    let first = true;
    let stop = false;
    let timer: ReturnType<typeof setTimeout>;
    const tick = async () => {
      try {
        const t = await api<JobTail>(`/jobs/${id}?offset=${offset}`);
        if (stop) return;
        offset = t.next;
        if (first) setLines(t.lines);
        else if (t.lines.length) setLines((l) => [...l, ...t.lines]);
        first = false;
        setJob(t);
        report.current?.(t.status);
        if (t.status !== "running") return;
      } catch {
        /* retry */
      }
      timer = setTimeout(tick, 1000);
    };
    tick();
    return () => {
      stop = true;
      clearTimeout(timer);
    };
  }, [id]);

  useEffect(() => {
    const el = box.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [lines]);

  return (
    <div className={cn("space-y-2", className)}>
      {job && (
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <JobStatus status={job.status} />
          <span>#{job.id}</span>
          <span className="font-mono">
            {job.kind}:{job.target}
          </span>
          <span>{job.username}</span>
          <span>{dateTime(job.started_at)}</span>
          {job.exit_code != null && job.exit_code !== 0 && <span className="text-destructive">код {job.exit_code}</span>}
        </div>
      )}
      <div
        ref={box}
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        className="h-[min(60vh,28rem)] overflow-auto rounded-lg border bg-zinc-950 p-3 font-mono text-xs leading-relaxed text-zinc-200"
      >
        {lines.length === 0 && <div className="text-zinc-500">Ожидание вывода…</div>}
        {lines.map((l, i) => (
          <div
            key={i}
            className={cn(
              "whitespace-pre-wrap break-all",
              l.startsWith("▶") && "mt-1 font-semibold text-sky-300",
              l.startsWith("✓") && "text-emerald-400",
              (l.startsWith("✗") || l.startsWith("error")) && "text-red-400",
            )}
          >
            {l}
          </div>
        ))}
      </div>
    </div>
  );
}

export function JobDialog({ job, onClose }: { job: { id: number; title: string } | null; onClose: () => void }) {
  return (
    <Dialog open={!!job} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle>{job?.title}</DialogTitle>
          <DialogDescription>Задача выполняется на сервере и продолжится, даже если закрыть окно. История — в журнале задач.</DialogDescription>
        </DialogHeader>
        {job && <JobLog id={job.id} />}
      </DialogContent>
    </Dialog>
  );
}
