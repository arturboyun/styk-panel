"use client";

import { type ReactNode, useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Archive, CalendarClock, ChevronDown, Clock, Database, Download, FileArchive, History, Play, RotateCcw, ScrollText, TerminalSquare, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { JobsTable } from "@/components/jobs-table";
import { Empty, PageHeader } from "@/components/page";
import { Dot } from "@/components/status";
import { UnitJournalSheet } from "@/components/unit-journal";
import { api, post } from "@/lib/api";
import { ago, bytes, duration, secondsSince } from "@/lib/format";
import { errorMessage, useAction, useDownload, useOps } from "@/lib/ops";
import type { BackupFile, BackupsOverview, PgServer, QueryResult, RedisServer, RestoreSchedule, Timer } from "@/lib/types";
import { cn } from "@/lib/utils";

const BACKUP_UNIT = "backup-all.service";
const RESTORE_TEST_UNIT = "backup-restore-test.service";
const WEEKDAYS = [
  ["Mon", "Пн"],
  ["Tue", "Вт"],
  ["Wed", "Ср"],
  ["Thu", "Чт"],
  ["Fri", "Пт"],
  ["Sat", "Сб"],
  ["Sun", "Вс"],
] as const;

function groupOf(path: string, dirs: string[]): string {
  const dir = dirs.find((d) => path.startsWith(d + "/")) ?? path.slice(0, path.lastIndexOf("/"));
  const rest = path.slice(dir.length + 1);
  return rest.includes("/") ? `${dir}/${rest.split("/")[0]}` : dir;
}

function BackupAllCard({ data }: { data: BackupsOverview }) {
  const act = useAction();
  const [journal, setJournal] = useState<string | null>(null);
  const u = data.unit;
  if (!u || u.LoadState !== "loaded")
    return (
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">backup-all</CardTitle>
          <CardDescription>Юнит {BACKUP_UNIT} не найден на этом сервере</CardDescription>
        </CardHeader>
      </Card>
    );
  const start = Number(u.ExecMainStartTimestamp) || 0;
  const end = Number(u.ExecMainExitTimestamp) || 0;
  const running = u.ActiveState === "activating";
  const okRun = u.Result === "success";
  const age = end ? secondsSince(end) : null;
  const stale = age != null && age > 26 * 3600;
  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle className="flex items-center gap-2">
            <Dot tone={running ? "warn" : okRun && !stale ? "ok" : "bad"} pulse={running} /> backup-all
          </CardTitle>
          <CardDescription>Ночной pg_dump всех Postgres-контейнеров, проверка pg_restore -l, копия на второй сервер и в R2</CardDescription>
        </div>
        <div className="flex gap-2">
          <Button variant="outline" size="sm" onClick={() => setJournal(BACKUP_UNIT)}>
            <ScrollText /> Журнал
          </Button>
          <Button
            size="sm"
            disabled={running}
            onClick={() => act(() => post(`/system/units/${BACKUP_UNIT}/run`), { job: "backup-all", invalidate: [["backups"]] })}
          >
            <Play /> Запустить сейчас
          </Button>
        </div>
      </CardHeader>
      <CardContent className="grid gap-4 sm:grid-cols-4">
        <div>
          <div className="text-xs text-muted-foreground">Последний запуск</div>
          <div className="mt-1 font-medium">{running ? "выполняется…" : end ? ago(end) : "—"}</div>
          {end > 0 && <div className="text-xs text-muted-foreground">{new Date(end * 1000).toLocaleString("ru")}</div>}
        </div>
        <div>
          <div className="text-xs text-muted-foreground">Результат</div>
          <div className={cn("mt-1 font-medium", okRun ? "text-success" : "text-destructive")}>{okRun ? "успешно" : u.Result || "—"}</div>
          {stale && <div className="text-xs text-destructive">старше 26 часов</div>}
        </div>
        <div>
          <div className="text-xs text-muted-foreground">Длительность</div>
          <div className="mt-1 font-medium tabular">{start && end >= start ? (end - start < 60 ? `${end - start} с` : duration(end - start)) : "—"}</div>
        </div>
        <div>
          <div className="text-xs text-muted-foreground">Следующий</div>
          <div className="mt-1 font-medium">{data.timer?.next ? ago(data.timer.next / 1e6) : "не запланирован"}</div>
          {data.timer?.next && <div className="text-xs text-muted-foreground">{new Date(data.timer.next / 1000).toLocaleString("ru")}</div>}
        </div>
      </CardContent>
      <UnitJournalSheet unit={journal} onClose={() => setJournal(null)} />
    </Card>
  );
}

function Stat({ label, children, sub, tone }: { label: string; children: ReactNode; sub?: ReactNode; tone?: "ok" | "bad" }) {
  return (
    <div>
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className={cn("mt-1 font-medium tabular", tone === "ok" && "text-success", tone === "bad" && "text-destructive")}>{children}</div>
      {sub && <div className="text-xs text-muted-foreground">{sub}</div>}
    </div>
  );
}

function R2Card({ data }: { data: BackupsOverview }) {
  const r2 = data.r2;
  const latest = r2?.latest;
  const stale = !latest || secondsSince(latest.ts) > 26 * 3600;
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Dot tone={!r2 ? "bad" : stale ? "warn" : "ok"} /> Копия в Cloudflare R2
        </CardTitle>
        <CardDescription>
          restic шифрует данные на сервере: дампы, /opt с .env, /etc, /root, uploads. Хранение — всё за 30 дней, 8 недель и 12 месяцев; Bucket Lock не даёт удалить
          данные раньше 30 дней.
        </CardDescription>
      </CardHeader>
      <CardContent className="grid gap-4 sm:grid-cols-4">
        {!r2 ? (
          <div className="text-sm text-muted-foreground sm:col-span-4">
            Нет /var/lib/backup-all/r2-status.json — backup-r2 ещё не запускался на этом сервере.
          </div>
        ) : (
          <>
            <Stat label="Последний снимок" sub={latest ? new Date(latest.ts * 1000).toLocaleString("ru") : undefined} tone={stale ? "bad" : undefined}>
              {latest ? ago(latest.ts) : "—"}
            </Stat>
            <Stat label="Снимков" sub={`бакет ${r2.repository}`}>
              {r2.snapshots}
            </Stat>
            <Stat label="Размер снимка" sub="до сжатия и дедупликации">
              {latest?.size != null ? bytes(latest.size) : "—"}
            </Stat>
            <Stat label="Занято в R2" sub="все снимки, сжато">
              {r2.stored_bytes != null ? bytes(r2.stored_bytes) : "—"}
            </Stat>
          </>
        )}
      </CardContent>
    </Card>
  );
}

function ScheduleDialog({ open, initial, onClose }: { open: boolean; initial: RestoreSchedule; onClose: () => void }) {
  const act = useAction();
  const [s, setS] = useState(initial);
  const toggle = (d: string, on: boolean) => setS({ ...s, days: on ? [...s.days, d] : s.days.filter((x) => x !== d) });
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Расписание проверки восстановления</DialogTitle>
          <DialogDescription>Таймер backup-restore-test.timer на сервере. Время в UTC; ночной бэкап идёт около 03:15–03:30.</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <label className="flex items-center justify-between gap-3 text-sm">
            Проверять по расписанию
            <Switch checked={s.enabled} onCheckedChange={(v) => setS({ ...s, enabled: v })} />
          </label>
          <div className={cn("space-y-4", !s.enabled && "pointer-events-none opacity-50")}>
            <div className="flex flex-wrap gap-3">
              {WEEKDAYS.map(([d, label]) => (
                <label key={d} className="flex items-center gap-1.5 text-sm">
                  <Checkbox checked={s.days.includes(d)} onCheckedChange={(v) => toggle(d, !!v)} /> {label}
                </label>
              ))}
            </div>
            <label className="flex items-center gap-3 text-sm">
              Время (UTC)
              <Input type="time" value={s.time} onChange={(e) => setS({ ...s, time: e.target.value })} className="w-32" />
            </label>
          </div>
          {!s.enabled && <p className="text-xs text-muted-foreground">Выключите и алерт «Проверка восстановления не прошла», иначе через 8 дней он сработает.</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Отмена
          </Button>
          <Button
            disabled={s.enabled && s.days.length === 0}
            onClick={async () => {
              const r = await act(() => post("/backups/restore-test/schedule", s), { success: "Расписание сохранено", invalidate: [["backups"]] });
              if (r) onClose();
            }}
          >
            Сохранить
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function RestoreTestCard({ data }: { data: BackupsOverview }) {
  const act = useAction();
  const [journal, setJournal] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const t = data.restore_test;
  const last = t.last;
  const running = t.unit?.ActiveState === "activating";
  const sched = t.schedule;
  const days = WEEKDAYS.filter(([d]) => sched.days.includes(d))
    .map(([, l]) => l)
    .join(", ");
  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle className="flex items-center gap-2">
            <Dot tone={running ? "warn" : !last ? "warn" : last.ok ? "ok" : "bad"} pulse={running} /> Проверка восстановления
          </CardTitle>
          <CardDescription>
            Берёт последний снимок из R2, поднимает каждый дамп во временном Postgres без сети и считает таблицы и строки. Проверяет, что .env на месте.
          </CardDescription>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" size="sm" onClick={() => setJournal(RESTORE_TEST_UNIT)}>
            <ScrollText /> Журнал
          </Button>
          <Button variant="outline" size="sm" onClick={() => setEditing(true)}>
            <CalendarClock /> Расписание
          </Button>
          <Button
            size="sm"
            disabled={running}
            onClick={() => act(() => post(`/system/units/${RESTORE_TEST_UNIT}/run`), { job: "Проверка восстановления", invalidate: [["backups"]] })}
          >
            <Play /> Проверить сейчас
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="grid gap-4 sm:grid-cols-4">
          <Stat label="Последняя проверка" sub={last ? new Date(last.ts * 1000).toLocaleString("ru") : undefined}>
            {running ? "выполняется…" : last ? ago(last.ts) : "ещё не было"}
          </Stat>
          <Stat label="Результат" tone={last ? (last.ok ? "ok" : "bad") : undefined} sub={last ? `за ${last.duration_s} с` : undefined}>
            {last ? (last.ok ? "восстанавливается" : "ошибка") : "—"}
          </Stat>
          <Stat label="Снимок" sub={last?.snapshot ? `сделан ${ago(last.snapshot.ts)}` : undefined}>
            <span className="font-mono">{last?.snapshot?.id ?? "—"}</span>
          </Stat>
          <Stat label="Расписание" sub={t.timer?.next ? `следующая ${ago(t.timer.next / 1e6)}` : undefined}>
            {sched.enabled ? `${days} · ${sched.time} UTC` : "выключено"}
          </Stat>
        </div>
        {last?.error && <div className="rounded-lg border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive">{last.error}</div>}
        {last && last.databases.length > 0 && (
          <div className="rounded-xl border">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-4">База</TableHead>
                  <TableHead>Дамп</TableHead>
                  <TableHead className="text-right">Таблиц</TableHead>
                  <TableHead className="pr-4 text-right">Строк</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {last.databases.map((d) => (
                  <TableRow key={d.container}>
                    <TableCell className="pl-4">
                      <div className="flex items-center gap-2 font-mono text-xs">
                        <Dot tone={d.ok ? "ok" : "bad"} /> {d.container}
                      </div>
                      {d.error && <div className="mt-1 max-w-md text-xs whitespace-normal text-destructive">{d.error}</div>}
                    </TableCell>
                    <TableCell className="font-mono text-xs text-muted-foreground">
                      {d.dump} · {bytes(d.size)}
                    </TableCell>
                    <TableCell className="text-right text-xs tabular">{d.tables}</TableCell>
                    <TableCell className="pr-4 text-right text-xs tabular">{d.rows.toLocaleString("ru")}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </CardContent>
      {editing && <ScheduleDialog open={editing} initial={sched} onClose={() => setEditing(false)} />}
      <UnitJournalSheet unit={journal} onClose={() => setJournal(null)} />
    </Card>
  );
}

function FilesCard({ data }: { data: BackupsOverview }) {
  const download = useDownload();
  const groups = useMemo(() => {
    const m = new Map<string, BackupFile[]>();
    for (const f of data.backups.files) {
      const g = groupOf(f.path, data.backups.dirs);
      m.set(g, [...(m.get(g) ?? []), f]);
    }
    return [...m.entries()].sort((a, b) => a[0].localeCompare(b[0]));
  }, [data]);
  if (groups.length === 0) return <Empty icon={<Archive />} title="Файлов бэкапов нет">Каталоги: {data.backups.dirs.join(", ")}</Empty>;
  return (
    <div className="space-y-2">
      {groups.map(([g, files]) => {
        const total = files.reduce((s, f) => s + f.size, 0);
        const newest = files[0];
        const old = secondsSince(newest.mtime) > 26 * 3600;
        return (
          <Collapsible key={g} className="rounded-xl border">
            <CollapsibleTrigger className="group flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-muted/40">
              <FileArchive className="size-4 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <div className="truncate font-mono text-sm">{g}</div>
                <div className="text-xs text-muted-foreground">
                  {files.length} файлов · {bytes(total)} · последний{" "}
                  <span className={cn(old && "text-warning")}>{ago(newest.mtime)}</span>
                </div>
              </div>
              <ChevronDown className="size-4 text-muted-foreground transition group-data-[panel-open]:rotate-180" />
            </CollapsibleTrigger>
            <CollapsibleContent>
              <div className="divide-y border-t">
                {files.slice(0, 60).map((f) => (
                  <div key={f.path} className="flex items-center gap-3 px-4 py-2 text-sm">
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">{f.path.split("/").pop()}</span>
                    <span className="w-20 text-right text-xs tabular text-muted-foreground">{bytes(f.size)}</span>
                    <span className="w-28 text-right text-xs text-muted-foreground">{ago(f.mtime)}</span>
                    <Button variant="ghost" size="icon-sm" onClick={() => download(f.path)} aria-label="Скачать">
                      <Download />
                    </Button>
                  </div>
                ))}
              </div>
            </CollapsibleContent>
          </Collapsible>
        );
      })}
      <p className="text-xs text-muted-foreground">Скачивание дампов требует подтверждения TOTP и пишется в аудит.</p>
    </div>
  );
}

function RestoreDialog({ server, db, files, onClose }: { server: PgServer; db: string | null; files: BackupFile[]; onClose: () => void }) {
  const act = useAction();
  const candidates = files.filter((f) => f.path.includes(server.container) && /\.dump$/.test(f.path));
  const [file, setFile] = useState<string>("");
  const [confirm, setConfirm] = useState("");
  const chosen = file || candidates[0]?.path || "";
  return (
    <Dialog open={!!db} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            Восстановить {server.container}/{db}
          </DialogTitle>
          <DialogDescription render={<div />}>
            <p>
              Перед восстановлением панель сделает дамп текущего состояния в /var/backups/styk-panel, затем выполнит{" "}
              <code className="font-mono">pg_restore --clean --if-exists --no-owner</code>. Текущие данные базы будут заменены.
            </p>
          </DialogDescription>
        </DialogHeader>
        {candidates.length === 0 ? (
          <div className="text-sm text-muted-foreground">Нет дампов для этого контейнера.</div>
        ) : (
          <div className="space-y-3">
            <Select value={chosen} onValueChange={(v) => setFile(String(v))} items={Object.fromEntries(candidates.map((c) => [c.path, c.path.split("/").pop()]))}>
              <SelectTrigger className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {candidates.map((c) => (
                  <SelectItem key={c.path} value={c.path}>
                    {c.path.split("/").pop()} · {bytes(c.size)} · {ago(c.mtime)}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
            <div className="space-y-1.5 text-sm">
              <div>
                Введите имя базы <code className="rounded bg-muted px-1 font-mono">{db}</code>
              </div>
              <Input value={confirm} onChange={(e) => setConfirm(e.target.value)} />
            </div>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Отмена
          </Button>
          <Button
            variant="destructive"
            disabled={!chosen || confirm !== db}
            onClick={async () => {
              const r = await act(() => post(`/databases/${server.container}/restore`, { database: db, file: chosen, confirm }), { job: `Восстановление ${db}` });
              if (r) onClose();
            }}
          >
            <RotateCcw /> Восстановить
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SqlConsole({ server, db, onClose }: { server: PgServer; db: string | null; onClose: () => void }) {
  const { sudo } = useOps();
  const [sql, setSql] = useState("SELECT now();");
  const [res, setRes] = useState<QueryResult | null>(null);
  const [busy, setBusy] = useState(false);
  const run = async () => {
    setBusy(true);
    try {
      setRes(await sudo(() => post<QueryResult>(`/databases/${server.container}/query`, { database: db, sql })));
    } catch (e) {
      setRes({ error: errorMessage(e) });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Sheet open={!!db} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="right" className="w-full gap-0 sm:max-w-4xl">
        <SheetHeader>
          <SheetTitle className="font-mono">
            {server.container}/{db}
          </SheetTitle>
          <SheetDescription>
            Транзакции только на чтение, таймаут 10 с, до 1000 строк. Ограничение можно снять командой SET, поэтому консоль требует sudo и пишет каждый запрос в аудит.
          </SheetDescription>
        </SheetHeader>
        <div className="space-y-3 p-4">
          <Textarea
            value={sql}
            onChange={(e) => setSql(e.target.value)}
            onKeyDown={(e) => (e.metaKey || e.ctrlKey) && e.key === "Enter" && run()}
            className="min-h-32 font-mono text-sm"
            spellCheck={false}
          />
          <div className="flex items-center gap-3">
            <Button onClick={run} disabled={busy}>
              <Zap /> Выполнить
            </Button>
            <span className="text-xs text-muted-foreground">Ctrl+Enter</span>
            {res?.ms != null && (
              <span className="ml-auto text-xs text-muted-foreground">
                {res.rows?.length} строк{res.truncated ? " (обрезано)" : ""} · {res.ms} мс
              </span>
            )}
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-auto border-t">
          {res?.error && <pre className="p-4 font-mono text-xs whitespace-pre-wrap text-destructive">{res.error}</pre>}
          {res?.columns && (
            <Table>
              <TableHeader>
                <TableRow>
                  {res.columns.map((c, i) => (
                    <TableHead key={i} className="font-mono text-xs">
                      {c}
                    </TableHead>
                  ))}
                </TableRow>
              </TableHeader>
              <TableBody>
                {res.rows?.map((r, i) => (
                  <TableRow key={i}>
                    {r.map((v, j) => (
                      <TableCell key={j} className="max-w-80 truncate font-mono text-xs" title={v}>
                        {v === "" ? <span className="text-muted-foreground">∅</span> : v}
                      </TableCell>
                    ))}
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
          {res?.notice && <pre className="p-4 font-mono text-xs text-muted-foreground">{res.notice}</pre>}
        </div>
      </SheetContent>
    </Sheet>
  );
}

function PgCard({ s, files }: { s: PgServer; files: BackupFile[] }) {
  const act = useAction();
  const [restore, setRestore] = useState<string | null>(null);
  const [console, setConsole] = useState<string | null>(null);
  const total = s.databases.reduce((a, d) => a + d.size, 0);
  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-2">
        <div>
          <CardTitle className="flex items-center gap-2">
            <Database className="size-4 text-primary" /> {s.container}
          </CardTitle>
          <CardDescription>
            PostgreSQL {s.version?.split(" ")[0] ?? "?"} · {s.project ?? "без проекта"} · пользователь {s.user}
          </CardDescription>
        </div>
        <div className="text-right text-xs text-muted-foreground">
          <div className="tabular">{bytes(total)}</div>
          <div>{s.connections ?? "—"} подключений</div>
        </div>
      </CardHeader>
      <CardContent className="px-0">
        {s.error ? (
          <div className="px-4 text-sm text-destructive">{s.error}</div>
        ) : (
          <Table>
            <TableBody>
              {s.databases
                .filter((d) => d.name !== "postgres" || s.databases.length === 1)
                .map((d) => (
                  <TableRow key={d.name}>
                    <TableCell className="pl-4 font-mono text-sm">{d.name}</TableCell>
                    <TableCell className="text-right text-xs tabular text-muted-foreground">{bytes(d.size)}</TableCell>
                    <TableCell className="text-right text-xs tabular text-muted-foreground">{d.connections} conn</TableCell>
                    <TableCell className="pr-4 text-right whitespace-nowrap">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => act(() => post(`/databases/${s.container}/dump`, { database: d.name }), { job: `Дамп ${d.name}`, invalidate: [["backups"]] })}
                      >
                        <Archive /> Дамп
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setRestore(d.name)}>
                        <History /> Restore
                      </Button>
                      <Button variant="ghost" size="sm" onClick={() => setConsole(d.name)}>
                        <TerminalSquare /> SQL
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
            </TableBody>
          </Table>
        )}
      </CardContent>
      <RestoreDialog server={s} db={restore} files={files} onClose={() => setRestore(null)} />
      <SqlConsole server={s} db={console} onClose={() => setConsole(null)} />
    </Card>
  );
}

function RedisCard({ r }: { r: RedisServer }) {
  const keys = Object.entries(r.info)
    .filter(([k]) => /^db\d+$/.test(k))
    .reduce((s, [, v]) => s + Number(/keys=(\d+)/.exec(v)?.[1] ?? 0), 0);
  const hits = Number(r.info.keyspace_hits ?? 0);
  const misses = Number(r.info.keyspace_misses ?? 0);
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Zap className="size-4 text-chart-3" /> {r.container}
        </CardTitle>
        <CardDescription>
          {r.image} · {r.project ?? "без проекта"}
        </CardDescription>
      </CardHeader>
      <CardContent>
        {r.error ? (
          <div className="text-sm text-destructive">{r.error}</div>
        ) : (
          <div className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-4">
            <div>
              <div className="text-xs text-muted-foreground">Память</div>
              <div className="font-medium tabular">{bytes(Number(r.info.used_memory))}</div>
              <div className="text-xs text-muted-foreground">{Number(r.info.maxmemory) ? `лимит ${bytes(Number(r.info.maxmemory))}` : "без лимита"}</div>
            </div>
            <div>
              <div className="text-xs text-muted-foreground">Ключей</div>
              <div className="font-medium tabular">{keys.toLocaleString("ru")}</div>
            </div>
            <div>
              <div className="text-xs text-muted-foreground">Hit rate</div>
              <div className="font-medium tabular">{hits + misses ? `${((hits / (hits + misses)) * 100).toFixed(0)}%` : "—"}</div>
            </div>
            <div>
              <div className="text-xs text-muted-foreground">Uptime</div>
              <div className="font-medium">{duration(Number(r.info.uptime_in_seconds ?? 0))}</div>
              <div className="text-xs text-muted-foreground">{r.info.connected_clients} клиентов</div>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

function TimersTab() {
  const { data, isLoading } = useQuery({ queryKey: ["timers"], queryFn: () => api<Timer[]>("/system/timers"), refetchInterval: 60_000 });
  const [journal, setJournal] = useState<string | null>(null);
  if (isLoading || !data) return <Skeleton className="h-96" />;
  const sorted = [...data].sort((a, b) => (a.next ?? Infinity) - (b.next ?? Infinity));
  return (
    <Card className="py-0">
      <CardContent className="px-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="pl-4">Таймер</TableHead>
              <TableHead>Запускает</TableHead>
              <TableHead>Последний</TableHead>
              <TableHead>Следующий</TableHead>
              <TableHead className="pr-4" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {sorted.map((t) => (
              <TableRow key={t.unit}>
                <TableCell className="pl-4 font-mono text-xs">{t.unit}</TableCell>
                <TableCell className="font-mono text-xs text-muted-foreground">{t.activates}</TableCell>
                <TableCell className="text-xs">{t.last ? ago(t.last / 1e6) : "—"}</TableCell>
                <TableCell className="text-xs">{t.next ? ago(t.next / 1e6) : "—"}</TableCell>
                <TableCell className="pr-4 text-right">
                  <Button variant="ghost" size="sm" onClick={() => setJournal(t.activates)}>
                    <ScrollText /> Журнал
                  </Button>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
      <UnitJournalSheet unit={journal} onClose={() => setJournal(null)} />
    </Card>
  );
}

export default function BackupsPage() {
  const { data, isLoading, error } = useQuery({ queryKey: ["backups"], queryFn: () => api<BackupsOverview>("/backups/overview"), refetchInterval: 30_000 });
  const dbs = useQuery({
    queryKey: ["databases"],
    queryFn: () => api<{ postgres: PgServer[]; redis: RedisServer[] }>("/databases"),
    refetchInterval: 60_000,
  });
  return (
    <>
      <PageHeader title="Бэкапы и базы данных" description="Ночные бэкапы, дампы по требованию, восстановление и состояние баз" />
      <Tabs defaultValue="backups">
        <TabsList className="mb-4">
          <TabsTrigger value="backups">
            <Archive /> Бэкапы
          </TabsTrigger>
          <TabsTrigger value="db">
            <Database /> Базы данных
          </TabsTrigger>
          <TabsTrigger value="timers">
            <Clock /> Расписания
          </TabsTrigger>
          <TabsTrigger value="jobs">Задачи</TabsTrigger>
        </TabsList>
        <TabsContent value="backups" className="space-y-4">
          {error ? <Empty title="Не удалось получить данные">{error.message}</Empty> : isLoading || !data ? <Skeleton className="h-64" /> : (
            <>
              <BackupAllCard data={data} />
              <R2Card data={data} />
              <RestoreTestCard data={data} />
              <FilesCard data={data} />
            </>
          )}
        </TabsContent>
        <TabsContent value="db" className="space-y-4">
          {dbs.isLoading || !dbs.data ? (
            <Skeleton className="h-64" />
          ) : (
            <>
              {dbs.data.postgres.length === 0 && <Empty icon={<Database />} title="Postgres-контейнеров нет" />}
              <div className="grid gap-4 xl:grid-cols-2">
                {dbs.data.postgres.map((s) => (
                  <PgCard key={s.container} s={s} files={data?.backups.files ?? []} />
                ))}
              </div>
              {dbs.data.redis.length > 0 && (
                <>
                  <h2 className="pt-2 text-sm font-medium text-muted-foreground">Redis / Valkey</h2>
                  <div className="grid gap-4 xl:grid-cols-2">
                    {dbs.data.redis.map((r) => (
                      <RedisCard key={r.container} r={r} />
                    ))}
                  </div>
                </>
              )}
            </>
          )}
        </TabsContent>
        <TabsContent value="timers">
          <TimersTab />
        </TabsContent>
        <TabsContent value="jobs">
          <JobsTable kinds={["pg", "unit"]} />
        </TabsContent>
      </Tabs>
    </>
  );
}
