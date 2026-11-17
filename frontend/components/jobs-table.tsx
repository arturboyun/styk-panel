"use client";

import { useQuery } from "@tanstack/react-query";
import { ListChecks } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { JobStatus } from "@/components/job-log";
import { Empty } from "@/components/page";
import { api } from "@/lib/api";
import { dateTime, duration, secondsSince } from "@/lib/format";
import { useOps } from "@/lib/ops";
import type { JobRow } from "@/lib/types";

/** Recent jobs, optionally filtered by kind and target; a row opens its log. */
export function JobsTable({ kinds, target }: { kinds?: string[]; target?: string }) {
  const { openJob } = useOps();
  const { data } = useQuery({
    queryKey: ["jobs", kinds, target],
    queryFn: () =>
      api<JobRow[]>(`/jobs?limit=50${kinds?.length ? `&kind=${encodeURIComponent(kinds.join(","))}` : ""}${target ? `&target=${encodeURIComponent(target)}` : ""}`),
    refetchInterval: 5_000,
  });
  if (!data?.length) return <Empty icon={<ListChecks />} title="Задач пока не было" />;
  return (
    <Card className="py-0">
      <CardContent className="px-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="pl-4">#</TableHead>
              <TableHead>Задача</TableHead>
              <TableHead>Статус</TableHead>
              <TableHead>Кто</TableHead>
              <TableHead>Начало</TableHead>
              <TableHead className="pr-4">Длительность</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.map((j) => (
              <TableRow key={j.id} className="cursor-pointer" onClick={() => openJob(j.id, `${j.kind}: ${j.target}`)}>
                <TableCell className="pl-4 text-xs text-muted-foreground tabular">{j.id}</TableCell>
                <TableCell className="font-mono text-xs">
                  {j.kind}:{j.target}
                </TableCell>
                <TableCell>
                  <JobStatus status={j.status} />
                </TableCell>
                <TableCell className="text-xs">{j.username}</TableCell>
                <TableCell className="text-xs text-muted-foreground">{dateTime(j.started_at)}</TableCell>
                <TableCell className="pr-4 text-xs tabular text-muted-foreground">
                  {j.finished_at ? Math.max(0, Math.round((new Date(j.finished_at).getTime() - new Date(j.started_at).getTime()) / 1000)) + " с" : duration(secondsSince(j.started_at))}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}
