"use client";

import Link from "next/link";
import { Suspense } from "react";
import { useSearchParams } from "next/navigation";
import { ChevronLeft, FileCode, FolderOpen, KeySquare, Lock, Pencil } from "lucide-react";
import { Button } from "@/components/ui/button";
import { JobsTable } from "@/components/jobs-table";
import { ContainerActions, HookCard, ProjectActions } from "@/components/project-actions";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Empty, PageHeader } from "@/components/page";
import { ContainerState, Dot, projectTone } from "@/components/status";
import { ago, bytes, pct } from "@/lib/format";
import { useContainerStats } from "@/lib/live";
import { useProjects } from "@/lib/queries";

function ProjectView() {
  const name = useSearchParams().get("name") ?? "";
  const { data, isLoading } = useProjects();
  const stats = useContainerStats();
  const p = data?.projects.find((x) => x.name === name);

  if (isLoading) return <Skeleton className="h-96" />;
  if (!p) return <Empty title={`Проект «${name}» не найден`} />;

  return (
    <>
      <Link href="/projects/" className="mb-3 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
        <ChevronLeft className="size-4" /> Проекты
      </Link>
      <PageHeader
        title={
          <span className="flex items-center gap-3">
            <Dot tone={projectTone(p)} className="scale-125" /> {p.name}
            {p.protected && (
              <Badge variant="outline" className="gap-1 font-normal">
                <Lock className="size-3" /> системный: остановка из панели запрещена
              </Badge>
            )}
          </span>
        }
        description={<span className="font-mono">{p.working_dir}</span>}
        actions={<ProjectActions project={p} />}
      />
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Контейнеры</CardTitle>
        </CardHeader>
        <CardContent className="px-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Сервис</TableHead>
                <TableHead>Состояние</TableHead>
                <TableHead className="text-right">CPU</TableHead>
                <TableHead className="text-right">RAM</TableHead>
                <TableHead className="hidden md:table-cell">Порты</TableHead>
                <TableHead className="hidden md:table-cell">Создан</TableHead>
                <TableHead className="w-10 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {p.containers.map((c) => {
                const s = stats.get(c.name);
                return (
                  <TableRow key={c.id}>
                    <TableCell className="pl-4">
                      <Link href={`/containers/view/?id=${c.id}`} className="font-medium hover:text-primary">
                        {c.service ?? c.name}
                      </Link>
                      <div className="max-w-xs truncate text-xs text-muted-foreground">{c.image}</div>
                    </TableCell>
                    <TableCell>
                      <ContainerState c={c} />
                      <div className="text-xs text-muted-foreground">{c.status}</div>
                    </TableCell>
                    <TableCell className="text-right tabular">{s ? pct(s.cpu_pct, 1) : "—"}</TableCell>
                    <TableCell className="text-right tabular">
                      {s ? bytes(s.mem_used) : "—"}
                      {s?.mem_limit && <div className="text-xs text-muted-foreground">из {bytes(s.mem_limit)}</div>}
                    </TableCell>
                    <TableCell className="hidden font-mono text-xs md:table-cell">{c.ports.join(", ") || "—"}</TableCell>
                    <TableCell className="hidden text-xs text-muted-foreground md:table-cell">{ago(c.created)}</TableCell>
                    <TableCell className="pr-4">
                      <ContainerActions c={c} protectedProject={p.protected} />
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
      <div className="mt-4 grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader className="flex flex-row items-start justify-between">
            <CardTitle className="text-sm">Конфигурация</CardTitle>
            {p.working_dir && (
              <Button variant="ghost" size="sm" render={<Link href={`/files/?path=${encodeURIComponent(p.working_dir)}`} />}>
                <FolderOpen /> Каталог
              </Button>
            )}
          </CardHeader>
          <CardContent className="space-y-1">
            {[...p.config_files, ...(p.working_dir ? [`${p.working_dir}/.env`] : [])].map((f) => (
              <Link
                key={f}
                href={`/files/edit/?path=${encodeURIComponent(f)}`}
                className="group flex items-center gap-2 rounded-md px-2 py-1.5 font-mono text-sm hover:bg-muted"
              >
                {f.endsWith(".env") ? <KeySquare className="size-4 text-muted-foreground" /> : <FileCode className="size-4 text-muted-foreground" />}
                <span className="min-w-0 flex-1 truncate">{f}</span>
                <Pencil className="size-3.5 opacity-0 transition group-hover:opacity-100" />
              </Link>
            ))}
            <p className="px-2 pt-2 text-xs text-muted-foreground">После правки compose или .env примените их кнопкой «Поднять» или «Pull + redeploy».</p>
          </CardContent>
        </Card>
        <HookCard project={p} />
      </div>
      <h2 className="mt-6 mb-3 text-sm font-medium text-muted-foreground">Операции</h2>
      <JobsTable kinds={["compose"]} target={p.name} />
    </>
  );
}

export default function Page() {
  return (
    <Suspense>
      <ProjectView />
    </Suspense>
  );
}
