"use client";

import Link from "next/link";
import { useMemo, useState } from "react";
import { AlertTriangle, ArrowDown, ArrowUp, Cpu, HardDrive, MemoryStick, Network } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Sparkline, TimeChart } from "@/components/charts/time-chart";
import { PageHeader } from "@/components/page";
import { Dot, Meter, projectTone } from "@/components/status";
import { bytes, duration, pct, rate } from "@/lib/format";
import { useLive } from "@/lib/live";
import { useHistory, useOverview, useProjects } from "@/lib/queries";
import type { HistoryPoint, Range, Sample } from "@/lib/types";
import { cn } from "@/lib/utils";

const RANGES: Range[] = ["1h", "6h", "24h", "7d", "30d"];

function StatCard({
  icon: Icon,
  title,
  value,
  sub,
  children,
}: {
  icon: React.ElementType;
  title: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <Card className="gap-3 overflow-hidden pb-0">
      <CardHeader className="flex flex-row items-center justify-between">
        <CardTitle className="text-sm font-medium text-muted-foreground">{title}</CardTitle>
        <Icon className="size-4 text-muted-foreground" />
      </CardHeader>
      <CardContent className="space-y-1 px-0">
        <div className="px-4">
          <div className="text-2xl font-semibold tracking-tight tabular">{value}</div>
          {sub && <div className="text-xs text-muted-foreground tabular">{sub}</div>}
        </div>
        <div className="pt-2">{children}</div>
      </CardContent>
    </Card>
  );
}

function toPoint(s: Sample): HistoryPoint {
  const root = s.filesystems.find((f) => f.mount === "/");
  return {
    ts: new Date(s.ts).toISOString(),
    cpu_pct: s.cpu_pct,
    mem_used: s.mem_used,
    mem_total: s.mem_total,
    load1: s.load[0],
    disk_used: root?.used ?? 0,
    disk_read_bps: s.disk_read_bps,
    disk_write_bps: s.disk_write_bps,
    net_rx_bps: s.net_rx_bps,
    net_tx_bps: s.net_tx_bps,
  };
}

function Charts({ range }: { range: Range }) {
  const { data, isLoading } = useHistory(range);
  const { samples } = useLive();

  const points = useMemo(() => {
    const base = data ?? [];
    if (range !== "1h") return base;
    const last = base.length ? new Date(base[base.length - 1].ts).getTime() : 0;
    return [...base, ...samples.filter((s) => s.ts > last).map(toPoint)];
  }, [data, samples, range]);

  const x = useMemo(() => points.map((p) => new Date(p.ts).getTime() / 1000), [points]);
  const col = <K extends keyof HistoryPoint>(k: K) => points.map((p) => p[k] as number);

  if (isLoading) return <Skeleton className="h-[460px] w-full" />;
  if (points.length < 2)
    return (
      <div className="rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">
        История за этот период ещё собирается: точки пишутся раз в минуту, часовые — раз в час.
      </div>
    );

  const memTotal = points[points.length - 1].mem_total;
  return (
    <div className="grid gap-4 md:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">CPU</CardTitle>
        </CardHeader>
        <CardContent>
          <TimeChart x={x} series={[{ label: "CPU", color: "--chart-1", values: col("cpu_pct") }]} yFormat={(v) => `${v.toFixed(0)}%`} yMax={100} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Память</CardTitle>
        </CardHeader>
        <CardContent>
          <TimeChart x={x} series={[{ label: "RAM", color: "--chart-2", values: col("mem_used") }]} yFormat={(v) => bytes(v, 0)} yMax={memTotal} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle className="text-sm">Сеть</CardTitle>
          <Legend items={[["--chart-5", "входящий"], ["--chart-3", "исходящий"]]} />
        </CardHeader>
        <CardContent>
          <TimeChart
            x={x}
            series={[
              { label: "rx", color: "--chart-5", values: col("net_rx_bps") },
              { label: "tx", color: "--chart-3", values: col("net_tx_bps"), fill: false },
            ]}
            yFormat={(v) => rate(v)}
          />
        </CardContent>
      </Card>
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle className="text-sm">Диск I/O</CardTitle>
          <Legend items={[["--chart-2", "чтение"], ["--chart-4", "запись"]]} />
        </CardHeader>
        <CardContent>
          <TimeChart
            x={x}
            series={[
              { label: "read", color: "--chart-2", values: col("disk_read_bps") },
              { label: "write", color: "--chart-4", values: col("disk_write_bps"), fill: false },
            ]}
            yFormat={(v) => rate(v)}
          />
        </CardContent>
      </Card>
    </div>
  );
}

function Legend({ items }: { items: [string, string][] }) {
  return (
    <div className="flex gap-3 text-xs text-muted-foreground">
      {items.map(([c, l]) => (
        <span key={l} className="flex items-center gap-1.5">
          <span className="size-2 rounded-full" style={{ background: `var(${c})` }} /> {l}
        </span>
      ))}
    </div>
  );
}

function TopContainers({ sample, by }: { sample: Sample; by: "cpu" | "mem" }) {
  const list = [...sample.containers].sort((a, b) => (by === "cpu" ? b.cpu_pct - a.cpu_pct : b.mem_used - a.mem_used)).slice(0, 7);
  const max = by === "cpu" ? Math.max(...list.map((c) => c.cpu_pct), 1) : Math.max(...list.map((c) => c.mem_used), 1);
  return (
    <div className="space-y-2.5">
      {list.map((c) => {
        const v = by === "cpu" ? c.cpu_pct : c.mem_used;
        return (
          <Link key={c.id} href={`/containers/view/?id=${c.id}`} className="group block">
            <div className="mb-1 flex items-baseline justify-between gap-2 text-sm">
              <span className="truncate group-hover:text-primary">{c.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground tabular">{by === "cpu" ? pct(v, 1) : bytes(v)}</span>
            </div>
            <div className="h-1 overflow-hidden rounded-full bg-muted">
              <div className={cn("h-full rounded-full", by === "cpu" ? "bg-chart-1" : "bg-chart-2")} style={{ width: `${(v / max) * 100}%` }} />
            </div>
          </Link>
        );
      })}
    </div>
  );
}

export default function Dashboard() {
  const { data: overview, isLoading } = useOverview(true);
  const { data: projects } = useProjects();
  const { samples, latest } = useLive();
  const [range, setRange] = useState<Range>("1h");
  const s = latest ?? overview?.sample ?? null;
  const info = overview?.info;
  const root = s?.filesystems.find((f) => f.mount === "/") ?? s?.filesystems[0];
  const tail = samples.slice(-60);
  const docker = overview?.docker;

  const counts = useMemo(() => {
    const ps = projects?.projects ?? [];
    return {
      total: ps.length,
      ok: ps.filter((p) => projectTone(p) === "ok").length,
      problems: ps.filter((p) => ["bad", "warn"].includes(projectTone(p))),
      containers: ps.reduce((n, p) => n + p.containers.length, 0) + (projects?.standalone.length ?? 0),
    };
  }, [projects]);

  return (
    <>
      <PageHeader
        title="Обзор"
        description={info ? `${info.hostname} · ${info.os} · ядро ${info.kernel} · ${info.cores} vCPU · аптайм ${duration(info.uptime_secs)}` : "…"}
      />

      {overview?.agent_error && (
        <Alert variant="destructive" className="mb-6">
          <AlertTriangle />
          <AlertTitle>styk-agent недоступен</AlertTitle>
          <AlertDescription>{overview.agent_error}</AlertDescription>
        </Alert>
      )}

      {isLoading || !s ? (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-36" />
          ))}
        </div>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard icon={Cpu} title="Процессор" value={pct(s.cpu_pct, 1)} sub={`load ${s.load.map((l) => l.toFixed(2)).join(" · ")}`}>
            <Sparkline values={tail.map((t) => t.cpu_pct)} max={100} />
          </StatCard>
          <StatCard
            icon={MemoryStick}
            title="Память"
            value={bytes(s.mem_used)}
            sub={`из ${bytes(s.mem_total)} · swap ${bytes(s.swap_used)} / ${bytes(s.swap_total)}`}
          >
            <Sparkline values={tail.map((t) => t.mem_used)} max={s.mem_total} color="--chart-2" />
          </StatCard>
          <StatCard
            icon={HardDrive}
            title={`Диск ${root?.mount ?? ""}`}
            value={root ? pct((root.used / root.total) * 100) : "—"}
            sub={root ? `${bytes(root.used)} из ${bytes(root.total)} · свободно ${bytes(root.total - root.used)}` : undefined}
          >
            <div className="px-4 pb-4">
              <Meter value={root ? (root.used / root.total) * 100 : 0} className="mt-2" />
              {root && root.inodes_total > 0 && (
                <div className="mt-2 text-xs text-muted-foreground tabular">inodes {pct((root.inodes_used / root.inodes_total) * 100, 1)}</div>
              )}
            </div>
          </StatCard>
          <StatCard
            icon={Network}
            title="Сеть"
            value={
              <span className="flex items-center gap-3 text-xl">
                <span className="flex items-center gap-1">
                  <ArrowDown className="size-4 text-chart-5" />
                  {rate(s.net_rx_bps)}
                </span>
                <span className="flex items-center gap-1">
                  <ArrowUp className="size-4 text-chart-3" />
                  {rate(s.net_tx_bps)}
                </span>
              </span>
            }
            sub={`диск: чтение ${rate(s.disk_read_bps)} · запись ${rate(s.disk_write_bps)}`}
          >
            <Sparkline values={tail.map((t) => t.net_rx_bps + t.net_tx_bps)} color="--chart-5" />
          </StatCard>
        </div>
      )}

      <div className="mt-8 mb-4 flex items-center justify-between">
        <h2 className="text-lg font-semibold tracking-tight">История</h2>
        <Tabs value={range} onValueChange={(v) => setRange(v as Range)}>
          <TabsList>
            {RANGES.map((r) => (
              <TabsTrigger key={r} value={r} className="px-3">
                {r}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      <Charts range={range} />

      <div className="mt-8 grid gap-4 lg:grid-cols-3">
        <Card>
          <CardHeader className="flex flex-row items-center justify-between">
            <CardTitle className="text-sm">Проекты</CardTitle>
            <Link href="/projects/" className="text-xs text-muted-foreground hover:text-foreground">
              все →
            </Link>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid grid-cols-3 gap-2">
              <div>
                <div className="text-2xl font-semibold tabular">{counts.total}</div>
                <div className="text-xs text-muted-foreground">проектов</div>
              </div>
              <div>
                <div className="text-2xl font-semibold tabular">{counts.containers}</div>
                <div className="text-xs text-muted-foreground">контейнеров</div>
              </div>
              <div>
                <div className={cn("text-2xl font-semibold tabular", counts.problems.length ? "text-warning" : "text-success")}>
                  {counts.problems.length}
                </div>
                <div className="text-xs text-muted-foreground">проблемных</div>
              </div>
            </div>
            <div className="space-y-1">
              {(projects?.projects ?? []).map((p) => (
                <Link
                  key={p.name}
                  href={`/projects/view/?name=${encodeURIComponent(p.name)}`}
                  className="flex items-center gap-2 rounded-md px-2 py-1 text-sm hover:bg-muted"
                >
                  <Dot tone={projectTone(p)} />
                  <span className="truncate">{p.name}</span>
                  <span className="ml-auto text-xs text-muted-foreground tabular">
                    {p.containers.filter((c) => c.state === "running").length}/{p.containers.length}
                  </span>
                </Link>
              ))}
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Топ по CPU</CardTitle>
          </CardHeader>
          <CardContent>{s ? <TopContainers sample={s} by="cpu" /> : <Skeleton className="h-48" />}</CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Топ по памяти</CardTitle>
          </CardHeader>
          <CardContent>{s ? <TopContainers sample={s} by="mem" /> : <Skeleton className="h-48" />}</CardContent>
        </Card>
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-3">
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle className="text-sm">Ядра процессора</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid grid-cols-4 gap-2 sm:grid-cols-8">
              {s?.cores.map((c, i) => (
                <div key={i} className="space-y-1">
                  <div className="relative h-16 overflow-hidden rounded-md bg-muted">
                    <div className="absolute inset-x-0 bottom-0 bg-chart-1/70 transition-[height] duration-500" style={{ height: `${Math.min(c, 100)}%` }} />
                  </div>
                  <div className="text-center text-[11px] text-muted-foreground tabular">
                    {i} · {c.toFixed(0)}%
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Docker на диске</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            {docker ? (
              <>
                <Row label={`Образы (${docker.images_count})`} value={bytes(docker.images_total)} hint={`${bytes(docker.images_reclaimable)} не используются`} />
                <Row label="Build cache" value={bytes(docker.build_cache_total)} hint={`${bytes(docker.build_cache_reclaimable)} можно освободить`} />
                <Row label={`Тома (${docker.volumes_count})`} value={bytes(docker.volumes_total)} />
                <Row label="Слои контейнеров" value={bytes(docker.containers_total)} />
                {docker.build_cache_reclaimable + docker.images_reclaimable > 5 * 2 ** 30 && (
                  <div className="rounded-lg bg-warning/10 px-3 py-2 text-xs text-warning">
                    Можно освободить ~{bytes(docker.build_cache_reclaimable + docker.images_reclaimable)}. Очистка появится в разделе проектов.
                  </div>
                )}
              </>
            ) : (
              <Skeleton className="h-32" />
            )}
          </CardContent>
        </Card>
      </div>

      {s && s.filesystems.length > 1 && (
        <Card className="mt-4">
          <CardHeader>
            <CardTitle className="text-sm">Файловые системы</CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            {s.filesystems.map((f) => (
              <div key={f.mount} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 gap-y-1 text-sm">
                <div className="truncate">
                  <span className="font-medium">{f.mount}</span> <span className="text-xs text-muted-foreground">{f.device} · {f.fstype}</span>
                </div>
                <div className="text-xs text-muted-foreground tabular">
                  {bytes(f.used)} / {bytes(f.total)}
                </div>
                <Meter value={(f.used / f.total) * 100} className="col-span-2" />
              </div>
            ))}
          </CardContent>
        </Card>
      )}
    </>
  );
}

function Row({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <div>
        <div>{label}</div>
        {hint && <div className="text-xs text-muted-foreground">{hint}</div>}
      </div>
      <div className="font-medium tabular">{value}</div>
    </div>
  );
}
