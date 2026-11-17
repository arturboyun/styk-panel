"use client";

import Link from "next/link";
import { Suspense, useMemo, useState } from "react";
import dynamic from "next/dynamic";
import { useRouter, useSearchParams } from "next/navigation";
import { ChevronLeft, Eye, EyeOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ContainerActions } from "@/components/project-actions";
import { api } from "@/lib/api";
import { useOps } from "@/lib/ops";
import type { EnvVar } from "@/lib/types";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { TimeChart } from "@/components/charts/time-chart";
import { LogViewer } from "@/components/log-viewer";
import { Empty, PageHeader } from "@/components/page";
import { ContainerState, Meter } from "@/components/status";
import { ApiError } from "@/lib/api";
import { ago, bytes, dateTime, pct } from "@/lib/format";
import { useLive } from "@/lib/live";
import { useContainer } from "@/lib/queries";

const ContainerTerminal = dynamic(() => import("@/components/terminal").then((m) => m.ContainerTerminal), { ssr: false });
const PROTECTED = ["styk-panel", "edge"];

function EnvCard({ id, env }: { id: string; env: EnvVar[] }) {
  const { sudo } = useOps();
  const [revealed, setRevealed] = useState<EnvVar[] | null>(null);
  const rows = revealed ?? env;
  const hidden = env.some((e) => e.masked);
  return (
    <Card className="lg:col-span-2">
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-sm">Переменные окружения</CardTitle>
        {hidden && (
          <Button
            variant="ghost"
            size="sm"
            onClick={async () => {
              if (revealed) return setRevealed(null);
              try {
                const r = await sudo(() => api<{ env: EnvVar[] }>(`/docker/containers/${encodeURIComponent(id)}?reveal=1`));
                setRevealed(r.env);
              } catch {}
            }}
          >
            {revealed ? <EyeOff /> : <Eye />} {revealed ? "Скрыть" : "Показать секреты"}
          </Button>
        )}
      </CardHeader>
      <CardContent className="max-h-96 overflow-auto font-mono text-xs">
        {rows.map((e) => (
          <div key={e.key} className="grid grid-cols-[minmax(10rem,auto)_minmax(0,1fr)] gap-3 py-0.5">
            <span className={e.secret ? "text-warning" : "text-primary"}>{e.key}</span>
            {e.masked ? (
              <span className="flex items-center gap-1 text-muted-foreground">
                <EyeOff className="size-3" /> скрыто
              </span>
            ) : (
              <span className="break-all">{e.value}</span>
            )}
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

function KV({ k, children }: { k: string; children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-[9rem_minmax(0,1fr)] gap-3 py-1.5 text-sm">
      <div className="text-muted-foreground">{k}</div>
      <div className="min-w-0 break-words">{children}</div>
    </div>
  );
}

function LiveMetrics({ name, memLimit }: { name: string; memLimit: number | null }) {
  const { samples } = useLive();
  const points = useMemo(
    () => samples.map((s) => ({ ts: s.ts / 1000, c: s.containers.find((c) => c.name === name) })).filter((p) => p.c),
    [samples, name],
  );
  const last = points[points.length - 1]?.c;
  if (points.length < 2)
    return <div className="rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">Собираю live-метрики, нужно несколько секунд (контейнер должен быть запущен).</div>;
  const x = points.map((p) => p.ts);
  const limit = last?.mem_limit ?? memLimit;
  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader className="flex flex-row justify-between">
          <CardTitle className="text-sm">CPU</CardTitle>
          <span className="text-sm tabular">{pct(last?.cpu_pct, 1)}</span>
        </CardHeader>
        <CardContent>
          <TimeChart x={x} series={[{ label: "CPU", color: "--chart-1", values: points.map((p) => p.c!.cpu_pct) }]} yFormat={(v) => `${v.toFixed(0)}%`} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="flex flex-row justify-between">
          <CardTitle className="text-sm">Память</CardTitle>
          <span className="text-sm tabular">
            {bytes(last?.mem_used)}
            {limit ? ` / ${bytes(limit)}` : ""}
          </span>
        </CardHeader>
        <CardContent>
          {limit && <Meter value={((last?.mem_used ?? 0) / limit) * 100} className="mb-3" />}
          <TimeChart x={x} series={[{ label: "RAM", color: "--chart-2", values: points.map((p) => p.c!.mem_used) }]} yFormat={(v) => bytes(v, 0)} />
        </CardContent>
      </Card>
    </div>
  );
}

function ContainerView() {
  const params = useSearchParams();
  const router = useRouter();
  const id = params.get("id") ?? "";
  const tab = params.get("tab") ?? "logs";
  const { data: c, error, isLoading } = useContainer(id);

  if (isLoading) return <Skeleton className="h-96" />;
  if (error instanceof ApiError && error.status === 404) return <Empty title="Контейнер не найден">Возможно, он был пересоздан. Вернитесь к списку проектов.</Empty>;
  if (!c) return null;

  const project = c.labels?.["com.docker.compose.project"];
  const setTab = (t: string) => router.replace(`/containers/view/?id=${id}&tab=${t}`, { scroll: false });

  return (
    <>
      <Link
        href={project ? `/projects/view/?name=${encodeURIComponent(project)}` : "/projects/"}
        className="mb-3 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
      >
        <ChevronLeft className="size-4" /> {project ?? "Проекты"}
      </Link>
      <PageHeader
        title={c.name}
        description={
          <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <ContainerState c={{ state: c.state.status, health: (c.state.health as "healthy" | null) ?? null }} />
            <span className="font-mono text-xs">{c.image}</span>
            {c.state.running && <span>запущен {ago(c.state.started_at)}</span>}
            {c.restart_count > 0 && <Badge variant="destructive">рестартов: {c.restart_count}</Badge>}
            {c.state.oom_killed && <Badge variant="destructive">OOM killed</Badge>}
          </span>
        }
        actions={
          <ContainerActions
            c={{ id: c.id, name: c.name, state: c.state.status, service: c.labels?.["com.docker.compose.service"] ?? null }}
            protectedProject={!!project && PROTECTED.includes(project)}
          />
        }
      />
      <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
        <TabsList className="mb-4">
          <TabsTrigger value="logs">Логи</TabsTrigger>
          <TabsTrigger value="metrics">Метрики</TabsTrigger>
          <TabsTrigger value="info">Конфигурация</TabsTrigger>
          <TabsTrigger value="terminal">Терминал</TabsTrigger>
        </TabsList>
        <TabsContent value="terminal">
          <ContainerTerminal containerId={id} running={c.state.running} />
        </TabsContent>
        <TabsContent value="logs">
          <LogViewer containerId={id} name={c.name} />
        </TabsContent>
        <TabsContent value="metrics">
          <LiveMetrics name={c.name} memLimit={c.memory_limit || null} />
        </TabsContent>
        <TabsContent value="info" className="grid gap-4 lg:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Состояние</CardTitle>
            </CardHeader>
            <CardContent className="divide-y">
              <KV k="Статус">{c.state.status}</KV>
              <KV k="Запущен">{c.state.started_at && !c.state.started_at.startsWith("0001") ? dateTime(c.state.started_at) : "—"}</KV>
              {!c.state.running && <KV k="Код выхода">{c.state.exit_code}</KV>}
              <KV k="Restart policy">{c.restart_policy || "—"}</KV>
              <KV k="Лимит памяти">{c.memory_limit ? bytes(c.memory_limit) : "нет"}</KV>
              <KV k="Лимит CPU">{c.nano_cpus ? `${c.nano_cpus / 1e9} CPU` : "нет"}</KV>
              <KV k="Сети">{c.networks?.join(", ") || "—"}</KV>
              <KV k="Команда">
                <code className="text-xs">{[...(c.entrypoint ?? []), ...(c.cmd ?? [])].join(" ") || "—"}</code>
              </KV>
              <KV k="ID">
                <code className="text-xs">{c.id.slice(0, 12)}</code>
              </KV>
            </CardContent>
          </Card>
          {c.state.health_log.length > 0 && (
            <Card>
              <CardHeader>
                <CardTitle className="text-sm">Healthcheck: {c.state.health}</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {c.state.health_log.map((h, i) => (
                  <div key={i} className="rounded-md bg-muted/60 p-2 text-xs">
                    <div className="mb-1 flex justify-between text-muted-foreground">
                      <span>{dateTime(h.end)}</span>
                      <span className={h.exit_code ? "text-destructive" : "text-success"}>exit {h.exit_code}</span>
                    </div>
                    <pre className="font-mono whitespace-pre-wrap">{h.output.trim() || "—"}</pre>
                  </div>
                ))}
              </CardContent>
            </Card>
          )}
          <Card>
            <CardHeader>
              <CardTitle className="text-sm">Тома и монтирования</CardTitle>
            </CardHeader>
            <CardContent className="space-y-2 text-sm">
              {c.mounts.length === 0 && <div className="text-muted-foreground">Нет</div>}
              {c.mounts.map((m) => (
                <div key={m.destination} className="font-mono text-xs">
                  <span className="text-muted-foreground">{m.source || m.type}</span> → {m.destination}
                  {!m.rw && <Badge variant="outline" className="ml-2">ro</Badge>}
                </div>
              ))}
            </CardContent>
          </Card>
          <EnvCard id={id} env={c.env} />
        </TabsContent>
      </Tabs>
    </>
  );
}

export default function Page() {
  return (
    <Suspense>
      <ContainerView />
    </Suspense>
  );
}
