"use client";

import { useState } from "react";
import { CheckCircle2, ScrollText, XCircle } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Empty, PageHeader } from "@/components/page";
import { dateTime } from "@/lib/format";
import { useAudit } from "@/lib/queries";

const ACTIONS: Record<string, string> = {
  "auth.login": "Вход",
  "auth.login_failed": "Неудачный вход",
  "auth.totp_failed": "Неверный TOTP",
  "auth.totp_enabled": "Включена 2FA",
  "auth.sudo": "Подтверждение sudo",
};

export default function AuditPage() {
  const [filter, setFilter] = useState("all");
  const { data, isLoading } = useAudit(filter === "all" ? undefined : filter);

  return (
    <>
      <PageHeader
        title="Аудит"
        description="Все входы и действия в панели"
        actions={
          <Select
            value={filter}
            onValueChange={(v) => setFilter(String(v))}
            items={{ all: "Все события", "auth.": "Авторизация", "auth.login_failed": "Неудачные входы", "docker.": "Docker" }}
          >
            <SelectTrigger className="w-48">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="all">Все события</SelectItem>
              <SelectItem value="auth.">Авторизация</SelectItem>
              <SelectItem value="auth.login_failed">Неудачные входы</SelectItem>
              <SelectItem value="docker.">Docker</SelectItem>
            </SelectContent>
          </Select>
        }
      />
      {isLoading ? (
        <Skeleton className="h-96" />
      ) : !data?.length ? (
        <Empty icon={<ScrollText />} title="Событий пока нет" />
      ) : (
        <Card className="py-0">
          <CardContent className="px-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-8 pl-4" />
                  <TableHead>Время</TableHead>
                  <TableHead>Событие</TableHead>
                  <TableHead>Пользователь</TableHead>
                  <TableHead>IP</TableHead>
                  <TableHead className="pr-4">Детали</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.map((r) => (
                  <TableRow key={r.id}>
                    <TableCell className="pl-4">
                      {r.ok ? <CheckCircle2 className="size-4 text-success" /> : <XCircle className="size-4 text-destructive" />}
                    </TableCell>
                    <TableCell className="text-xs whitespace-nowrap text-muted-foreground tabular">{dateTime(r.ts)}</TableCell>
                    <TableCell>
                      {ACTIONS[r.action] ?? r.action}
                      {r.target && <span className="ml-1 font-mono text-xs text-muted-foreground">{r.target}</span>}
                    </TableCell>
                    <TableCell>{r.username ?? "—"}</TableCell>
                    <TableCell className="font-mono text-xs">{r.ip}</TableCell>
                    <TableCell className="max-w-xs truncate pr-4 font-mono text-xs text-muted-foreground">
                      {Object.entries(r.details)
                        .filter(([, v]) => v !== null && v !== false)
                        .map(([k, v]) => `${k}=${typeof v === "string" ? v : JSON.stringify(v)}`)
                        .join(" ")}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
      )}
    </>
  );
}
