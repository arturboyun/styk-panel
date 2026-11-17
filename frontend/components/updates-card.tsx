"use client";

import { useEffect, useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpCircle, CheckCircle2, ExternalLink, Loader2, RefreshCw, XCircle } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useConfirm } from "@/components/confirm";
import { api, post } from "@/lib/api";
import { dateTime } from "@/lib/format";
import { useAction } from "@/lib/ops";
import type { VersionInfo } from "@/lib/types";

const v = (s: string) => (s.startsWith("v") ? s : `v${s}`);

/** Current and newest version; runs `styk update` on the server through the agent. */
export function UpdatesCard() {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const act = useAction();
  const { data, isLoading, isFetching } = useQuery({
    queryKey: ["version"],
    queryFn: () => api<VersionInfo>("/system/version"),
    staleTime: 30 * 60_000,
    // While an update runs the panel restarts under us: poll and keep the last answer.
    refetchInterval: (q) => (q.state.data?.update?.running ? 2_000 : false),
    retry: true,
  });
  const running = !!data?.update?.running;
  const wasRunning = useRef(false);
  useEffect(() => {
    // The new frontend is served once the update is done: reload to pick it up.
    if (wasRunning.current && !running && data?.update?.last?.state === "ok") location.reload();
    wasRunning.current = running;
  }, [running, data]);

  const start = async () => {
    const to = data?.latest?.version;
    if (!to) return;
    const yes = await confirm({
      title: `Обновить до ${to}?`,
      description: "Панель и агент перезапустятся, это около минуты. Перед обновлением сохраняется дамп базы панели; если новая версия не поднимется, вернётся текущая.",
      confirmText: "Обновить",
    });
    if (!yes) return;
    const r = await act(() => post("/system/update", { version: to }));
    if (r) qc.setQueryData<VersionInfo>(["version"], (d) => d && d.update ? { ...d, update: { ...d.update, running: true } } : d);
  };

  const u = data?.update;
  const last = u?.last;
  return (
    <Card className="lg:col-span-3">
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div className="space-y-1.5">
          <CardTitle>Версия и обновления</CardTitle>
          <CardDescription>Релизы проверяются раз в 6 часов{data?.repo ? ` в github.com/${data.repo}` : ""}; на сервере то же делает <code className="font-mono">styk update</code></CardDescription>
        </div>
        <Button variant="outline" size="sm" disabled={isFetching} onClick={() => qc.fetchQuery({ queryKey: ["version"], queryFn: () => api<VersionInfo>("/system/version?check=1"), staleTime: 0 })}>
          <RefreshCw className={isFetching ? "animate-spin" : ""} /> Проверить
        </Button>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        {isLoading && <Skeleton className="h-16" />}
        {data && (
          <div className="flex flex-wrap items-center gap-x-6 gap-y-2">
            <div>
              <span className="text-muted-foreground">Панель </span>
              <span className="font-mono font-medium">{v(data.panel)}</span>
            </div>
            {u && (
              <div>
                <span className="text-muted-foreground">Агент </span>
                <span className="font-mono font-medium">{v(u.agent_version)}</span>
              </div>
            )}
            {data.latest && (
              <div className="flex items-center gap-1.5">
                <span className="text-muted-foreground">Последний релиз </span>
                <a href={data.latest.url} target="_blank" rel="noreferrer" className="inline-flex items-center gap-1 font-mono font-medium hover:underline">
                  {data.latest.version} <ExternalLink className="size-3" />
                </a>
                <span className="text-xs text-muted-foreground">{dateTime(data.latest.published_at)}</span>
              </div>
            )}
            <div className="ml-auto">
              {running ? (
                <Badge variant="outline" className="gap-1">
                  <Loader2 className="size-3 animate-spin" /> обновляется
                </Badge>
              ) : data.update_available ? (
                <Button size="sm" onClick={start} disabled={!u?.cli}>
                  <ArrowUpCircle /> Обновить до {data.latest?.version}
                </Button>
              ) : data.latest ? (
                <Badge variant="outline" className="gap-1 border-success/40 text-success">
                  <CheckCircle2 className="size-3" /> актуальная версия
                </Badge>
              ) : null}
            </div>
          </div>
        )}
        {data && u && !u.cli && (
          <p className="text-xs text-muted-foreground">
            Сервер ставился не через <code className="font-mono">install.sh</code>, поэтому обновление из панели недоступно.
          </p>
        )}
        {data && !u && <p className="text-xs text-muted-foreground">Агент старой версии: обновите его через <code className="font-mono">styk update</code> на сервере.</p>}
        {last && !running && last.state === "failed" && (
          <div className="flex items-center gap-1.5 text-destructive">
            <XCircle className="size-4" /> Обновление {last.from} → {last.to} не удалось: {last.error}
          </div>
        )}
        {u?.log && (running || last?.state === "failed") && (
          <pre className="max-h-72 overflow-auto rounded-lg border bg-zinc-950 p-3 font-mono text-xs whitespace-pre-wrap text-zinc-200">{u.log}</pre>
        )}
        {data?.update_available && data.latest?.notes && !running && (
          <details className="rounded-lg border p-3">
            <summary className="cursor-pointer text-xs font-medium">Что нового в {data.latest.version}</summary>
            <pre className="mt-2 font-sans text-xs whitespace-pre-wrap text-muted-foreground">{data.latest.notes}</pre>
          </details>
        )}
      </CardContent>
    </Card>
  );
}
