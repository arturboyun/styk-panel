"use client";

import Link from "next/link";
import { useState } from "react";
import { Boxes, FolderOpen, Lock, Search } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Empty, PageHeader } from "@/components/page";
import { PruneButton } from "@/components/project-actions";
import { ImageRetentionButton } from "@/components/image-retention";
import { ContainerState, Dot, projectTone } from "@/components/status";
import { bytes, pct } from "@/lib/format";
import { useContainerStats } from "@/lib/live";
import { useProjects } from "@/lib/queries";
import type { Container } from "@/lib/types";

function ContainerRow({ c }: { c: Container }) {
  const stats = useContainerStats().get(c.name);
  return (
    <Link href={`/containers/view/?id=${c.id}`} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 rounded-md px-2 py-1.5 hover:bg-muted sm:grid-cols-[minmax(0,1fr)_auto_3.5rem_4.5rem]">
      <div className="min-w-0">
        <div className="truncate text-sm">{c.service ?? c.name}</div>
        <div className="truncate text-xs text-muted-foreground">{c.image}</div>
      </div>
      <ContainerState c={c} />
      <span className="hidden text-right text-xs text-muted-foreground tabular sm:block">{stats ? pct(stats.cpu_pct, 1) : ""}</span>
      <span className="hidden text-right text-xs text-muted-foreground tabular sm:block">{stats ? bytes(stats.mem_used) : ""}</span>
    </Link>
  );
}

export default function ProjectsPage() {
  const { data, isLoading } = useProjects();
  const stats = useContainerStats();
  const [q, setQ] = useState("");
  const needle = q.trim().toLowerCase();
  const match = (c: Container) => !needle || c.name.toLowerCase().includes(needle) || c.image.toLowerCase().includes(needle);
  const projects = (data?.projects ?? []).filter((p) => !needle || p.name.toLowerCase().includes(needle) || p.containers.some(match));
  const standalone = (data?.standalone ?? []).filter(match);

  return (
    <>
      <PageHeader
        title="Проекты"
        description="Compose-проекты и контейнеры на сервере"
        actions={
          <>
            <ImageRetentionButton />
            <PruneButton />
            <div className="relative">
              <Search className="absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Фильтр" className="w-56 pl-8" />
            </div>
          </>
        }
      />
      {isLoading ? (
        <div className="grid gap-4 xl:grid-cols-2">
          {Array.from({ length: 4 }, (_, i) => (
            <Skeleton key={i} className="h-56" />
          ))}
        </div>
      ) : projects.length === 0 && standalone.length === 0 ? (
        <Empty icon={<Boxes />} title="Ничего не найдено" />
      ) : (
        <div className="grid gap-4 xl:grid-cols-2">
          {projects.map((p) => {
            const cpu = p.containers.reduce((n, c) => n + (stats.get(c.name)?.cpu_pct ?? 0), 0);
            const mem = p.containers.reduce((n, c) => n + (stats.get(c.name)?.mem_used ?? 0), 0);
            return (
              <Card key={p.name} className="gap-3">
                <CardHeader className="flex flex-row items-start justify-between gap-3">
                  <div className="min-w-0">
                    <Link href={`/projects/view/?name=${encodeURIComponent(p.name)}`} className="flex items-center gap-2 font-semibold hover:text-primary">
                      <Dot tone={projectTone(p)} />
                      {p.name}
                      {p.protected && (
                        <Badge variant="outline" className="gap-1 font-normal">
                          <Lock className="size-3" /> системный
                        </Badge>
                      )}
                    </Link>
                    {p.working_dir && (
                      <div className="mt-1 flex items-center gap-1 truncate font-mono text-xs text-muted-foreground">
                        <FolderOpen className="size-3 shrink-0" /> {p.working_dir}
                      </div>
                    )}
                  </div>
                  <div className="shrink-0 text-right text-xs text-muted-foreground tabular">
                    <div>CPU {pct(cpu, 1)}</div>
                    <div>RAM {bytes(mem)}</div>
                  </div>
                </CardHeader>
                <CardContent className="space-y-0.5 px-2">
                  {p.containers.filter((c) => !needle || p.name.toLowerCase().includes(needle) || match(c)).map((c) => (
                    <ContainerRow key={c.id} c={c} />
                  ))}
                </CardContent>
              </Card>
            );
          })}
          {standalone.length > 0 && (
            <Card className="gap-3">
              <CardHeader>
                <div className="font-semibold">Без compose</div>
                <div className="text-xs text-muted-foreground">Контейнеры, запущенные через docker run</div>
              </CardHeader>
              <CardContent className="space-y-0.5 px-2">
                {standalone.map((c) => (
                  <ContainerRow key={c.id} c={c} />
                ))}
              </CardContent>
            </Card>
          )}
        </div>
      )}
    </>
  );
}
