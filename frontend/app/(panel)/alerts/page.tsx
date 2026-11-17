"use client";

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BellOff, BellRing, Check, CheckCircle2, Globe, HeartPulse, Plus, Send, Trash2 } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Textarea } from "@/components/ui/textarea";
import { useConfirm } from "@/components/confirm";
import { Empty, PageHeader } from "@/components/page";
import { Dot } from "@/components/status";
import { api, post } from "@/lib/api";
import { ago, dateTime, isFuture, pct } from "@/lib/format";
import { useAction } from "@/lib/ops";
import { cn } from "@/lib/utils";
import type { AlertEvent, AlertPrefs, AlertRule, AlertsResp, UptimeCheck } from "@/lib/types";

const SEVERITY: Record<AlertEvent["severity"], { label: string; cls: string }> = {
  critical: { label: "critical", cls: "bg-destructive/15 text-destructive" },
  warning: { label: "warning", cls: "bg-warning/15 text-warning" },
  info: { label: "info", cls: "bg-muted text-muted-foreground" },
};

const PARAM: Record<string, string> = {
  threshold: "порог, %",
  minutes: "минут подряд",
  warning: "warning, %",
  critical: "critical, %",
  count: "рестартов за 10 мин",
  days: "дней до истечения",
  failures: "неудач подряд",
  hours: "часов без бэкапа",
  per_hour: "попыток в час",
  gb: "ГБ",
  ignore_keys: "игнорировать ключи (по строке)",
};

function Sev({ s }: { s: AlertEvent["severity"] }) {
  return <span className={cn("rounded px-1.5 py-0.5 text-[11px] font-medium", SEVERITY[s].cls)}>{SEVERITY[s].label}</span>;
}

function EventRow({ e, active }: { e: AlertEvent; active?: boolean }) {
  const act = useAction();
  const inv = { invalidate: [["alerts"]] };
  const muted = isFuture(e.muted_until);
  return (
    <TableRow>
      <TableCell className="pl-4">
        <Sev s={e.severity} />
      </TableCell>
      <TableCell className="min-w-64 whitespace-normal">
        <div className="text-sm">{e.message}</div>
        <div className="text-xs text-muted-foreground">
          {e.rule} · {e.subject}
        </div>
      </TableCell>
      <TableCell className="text-xs whitespace-nowrap text-muted-foreground">
        {active ? ago(e.started_at) : dateTime(e.started_at)}
        {e.resolved_at && <div>→ {dateTime(e.resolved_at)}</div>}
      </TableCell>
      <TableCell className="text-xs text-muted-foreground">
        {e.acked_by && (
          <span className="inline-flex items-center gap-1">
            <Check className="size-3" /> {e.acked_by}
          </span>
        )}
        {muted && (
          <span className="inline-flex items-center gap-1">
            <BellOff className="size-3" /> до {dateTime(e.muted_until!)}
          </span>
        )}
      </TableCell>
      <TableCell className="pr-4 text-right whitespace-nowrap">
        {active && (
          <>
            {!e.acked_by && (
              <Button variant="ghost" size="sm" onClick={() => act(() => post(`/alerts/events/${e.id}/ack`), inv)}>
                <Check /> Ack
              </Button>
            )}
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button variant="ghost" size="sm" />}>
                <BellOff /> Mute
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                {[1, 6, 24, 168].map((h) => (
                  <DropdownMenuItem key={h} onClick={() => act(() => post(`/alerts/events/${e.id}/mute`, { hours: h }), inv)}>
                    {h < 24 ? `${h} ч` : h === 24 ? "сутки" : "неделя"}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        )}
      </TableCell>
    </TableRow>
  );
}

function Events({ events, active }: { events: AlertEvent[]; active?: boolean }) {
  if (!events.length)
    return active ? (
      <Empty icon={<CheckCircle2 />} title="Всё спокойно">
        Активных алертов нет.
      </Empty>
    ) : (
      <Empty title="История пуста" />
    );
  return (
    <Card className="py-0">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="w-24 pl-4">Уровень</TableHead>
            <TableHead>Событие</TableHead>
            <TableHead>Время</TableHead>
            <TableHead />
            <TableHead className="pr-4" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {events.map((e) => (
            <EventRow key={e.id} e={e} active={active} />
          ))}
        </TableBody>
      </Table>
    </Card>
  );
}

function RuleCard({ r }: { r: AlertRule }) {
  const act = useAction();
  const [params, setParams] = useState<Record<string, unknown>>(r.params);
  const dirty = JSON.stringify(params) !== JSON.stringify(r.params);
  const save = (enabled: boolean, p = params) => act(() => post(`/alerts/rules/${r.def.key}`, { enabled, params: p }), { success: "Сохранено", invalidate: [["alerts"]] });
  const muted = isFuture(r.muted_until);
  const keys = Object.keys(r.def.defaults);
  return (
    <div className="space-y-2 py-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2 text-sm font-medium">
            {r.def.title} <Sev s={r.def.severity} />
            {r.def.kind === "event" && <Badge variant="outline">событие</Badge>}
            {muted && (
              <Badge variant="secondary">
                <BellOff /> до {dateTime(r.muted_until!)}
              </Badge>
            )}
          </div>
          <div className="text-xs text-muted-foreground">{r.def.description}</div>
        </div>
        <div className="flex items-center gap-1">
          <DropdownMenu>
            <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Mute" />}>
              <BellOff />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              {[1, 24, 168, 0].map((h) => (
                <DropdownMenuItem key={h} onClick={() => act(() => post(`/alerts/rules/${r.def.key}/mute`, { hours: h }), { invalidate: [["alerts"]] })}>
                  {h === 0 ? "Снять mute" : h === 1 ? "1 ч" : h === 24 ? "сутки" : "неделя"}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
          <Switch checked={r.enabled} onCheckedChange={(v) => save(v, r.params)} />
        </div>
      </div>
      {keys.length > 0 && (
        <div className="flex flex-wrap items-end gap-3">
          {keys.map((k) =>
            Array.isArray(r.def.defaults[k]) ? (
              <div key={k} className="w-full space-y-1">
                <Label className="text-xs text-muted-foreground">{PARAM[k] ?? k}</Label>
                <Textarea
                  rows={2}
                  className="font-mono text-xs"
                  value={((params[k] as string[]) ?? []).join("\n")}
                  onChange={(e) => setParams({ ...params, [k]: e.target.value.split("\n").map((s) => s.trim()).filter(Boolean) })}
                  placeholder="SHA256:…"
                />
              </div>
            ) : (
              <div key={k} className="space-y-1">
                <Label className="text-xs text-muted-foreground">{PARAM[k] ?? k}</Label>
                <Input type="number" className="h-8 w-28" value={String(params[k] ?? "")} onChange={(e) => setParams({ ...params, [k]: Number(e.target.value) })} />
              </div>
            ),
          )}
          {dirty && (
            <Button size="sm" onClick={() => save(r.enabled)}>
              Сохранить
            </Button>
          )}
        </div>
      )}
    </div>
  );
}

function Rules({ rules }: { rules: AlertRule[] }) {
  const groups = [...new Set(rules.map((r) => r.def.group))];
  return (
    <div className="grid gap-4 xl:grid-cols-2">
      {groups.map((g) => (
        <Card key={g}>
          <CardHeader>
            <CardTitle className="text-sm">{g}</CardTitle>
          </CardHeader>
          <CardContent className="divide-y">
            {rules
              .filter((r) => r.def.group === g)
              .map((r) => (
                <RuleCard key={r.def.key} r={r} />
              ))}
          </CardContent>
        </Card>
      ))}
    </div>
  );
}

function Bars({ recent }: { recent: boolean[] | null }) {
  const cells = recent ?? [];
  return (
    <div className="flex h-5 items-stretch gap-px" title="последние проверки, слева старые">
      {Array.from({ length: 60 - cells.length }, (_, i) => (
        <span key={`e${i}`} className="w-1 rounded-[1px] bg-muted" />
      ))}
      {cells.map((u, i) => (
        <span key={i} className={cn("w-1 rounded-[1px]", u ? "bg-success/70" : "bg-destructive")} />
      ))}
    </div>
  );
}

function Uptime() {
  const act = useAction();
  const confirm = useConfirm();
  const qc = useQueryClient();
  const { data, isLoading } = useQuery({ queryKey: ["uptime"], queryFn: () => api<UptimeCheck[]>("/uptime"), refetchInterval: 30_000 });
  const [url, setUrl] = useState("");
  const inv = { invalidate: [["uptime"]] };
  if (isLoading) return <Skeleton className="h-96" />;
  return (
    <div className="space-y-4">
      <form
        className="flex flex-wrap gap-2"
        onSubmit={async (e) => {
          e.preventDefault();
          const r = await act(() => post("/uptime", { url }), { success: "Проверка добавлена", ...inv });
          if (r) setUrl("");
        }}
      >
        <Input value={url} onChange={(e) => setUrl(e.target.value)} placeholder="https://example.com/health" className="max-w-md" />
        <Button type="submit" variant="outline" disabled={!url}>
          <Plus /> Добавить URL
        </Button>
      </form>
      {!data?.length ? (
        <Empty icon={<Globe />} title="Проверок нет">
          Домены из Caddyfile добавляются автоматически каждые 10 минут.
        </Empty>
      ) : (
        <Card className="py-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">URL</TableHead>
                <TableHead className="hidden lg:table-cell">Последний час</TableHead>
                <TableHead className="text-right">24 ч</TableHead>
                <TableHead className="hidden text-right md:table-cell">7 д</TableHead>
                <TableHead className="hidden text-right md:table-cell">Latency</TableHead>
                <TableHead className="w-24 pr-4" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.map((c) => (
                <TableRow key={c.id} className={cn(!c.enabled && "opacity-50")}>
                  <TableCell className="pl-4">
                    <div className="flex items-center gap-2">
                      <Dot tone={!c.enabled || c.up == null ? "idle" : c.up ? "ok" : "bad"} pulse={c.enabled && c.up === false} />
                      <a href={c.url} target="_blank" rel="noreferrer" className="font-medium hover:underline">
                        {c.name || c.url.replace(/^https?:\/\//, "")}
                      </a>
                      {c.source === "caddy" && <Badge variant="outline">caddy</Badge>}
                    </div>
                    <div className="pl-4 text-xs text-muted-foreground">
                      {c.error ? <span className="text-destructive">{c.error}</span> : c.status ? `HTTP ${c.status}` : "ещё не проверялся"}
                      {c.last_ts && ` · ${ago(c.last_ts)}`}
                    </div>
                  </TableCell>
                  <TableCell className="hidden lg:table-cell">
                    <Bars recent={c.recent} />
                  </TableCell>
                  <TableCell className="text-right tabular">{pct(c.uptime_24h, 2)}</TableCell>
                  <TableCell className="hidden text-right tabular md:table-cell">{pct(c.uptime_7d, 2)}</TableCell>
                  <TableCell className="hidden text-right text-muted-foreground tabular md:table-cell">{c.avg_latency_24h != null ? `${Math.round(c.avg_latency_24h)} мс` : "—"}</TableCell>
                  <TableCell className="pr-4">
                    <div className="flex items-center justify-end gap-1">
                      <Switch size="sm" checked={c.enabled} onCheckedChange={(v) => act(() => post(`/uptime/${c.id}/toggle`, { enabled: v }), inv)} />
                      {c.source === "manual" && (
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          aria-label="Удалить"
                          onClick={async () => {
                            if (!(await confirm({ title: `Удалить проверку ${c.url}?`, destructive: true, confirmText: "Удалить" }))) return;
                            await act(() => api(`/uptime/${c.id}`, { method: "DELETE" }), inv);
                            qc.invalidateQueries({ queryKey: ["uptime"] });
                          }}
                        >
                          <Trash2 />
                        </Button>
                      )}
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}
    </div>
  );
}

const hour = (h: number) => `${String(h).padStart(2, "0")}:00`;

function HourInput({ value, onChange }: { value: number; onChange: (v: number) => void }) {
  return <Input type="number" min={0} max={23} className="h-8 w-20" value={value} onChange={(e) => onChange(Math.max(0, Math.min(23, Number(e.target.value))))} />;
}

function Notifications({ data }: { data: AlertsResp }) {
  const act = useAction();
  const [prefs, setPrefs] = useState<AlertPrefs>(data.prefs);
  const [chat, setChat] = useState(data.telegram.chat_id?.toString() ?? "");
  const tg = data.telegram;
  const tz = -new Date().getTimezoneOffset();
  const dirty = JSON.stringify(prefs) !== JSON.stringify(data.prefs) || chat !== (tg.chat_id?.toString() ?? "") || prefs.tz_offset_min !== tz;
  const set = <K extends keyof AlertPrefs>(k: K, v: AlertPrefs[K]) => setPrefs((p) => ({ ...p, [k]: v }));
  const save = () =>
    act(() => post("/alerts/settings", { prefs: { ...prefs, tz_offset_min: tz }, chat_id: chat ? Number(chat) : null }), { success: "Настройки сохранены", invalidate: [["alerts"]] });
  return (
    <div className="grid gap-4 xl:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-sm">
            <Send className="size-4" /> Telegram
          </CardTitle>
          <CardDescription>
            {tg.configured ? (
              <>Бот @{tg.bot_username}. Кнопки в уведомлениях (Ack, Mute) принимаются только от привязанных Telegram-аккаунтов.</>
            ) : (
              <>Бот не настроен: задайте TELEGRAM_BOT_TOKEN и TELEGRAM_BOT_USERNAME в /opt/styk-panel/.env и перезапустите панель. Без бота алерты видны только здесь.</>
            )}
          </CardDescription>
        </CardHeader>
        {tg.configured && (
          <CardContent className="space-y-4 text-sm">
            <div className="space-y-1.5">
              <Label htmlFor="chat">Chat ID</Label>
              <div className="flex gap-2">
                <Input id="chat" value={chat} onChange={(e) => setChat(e.target.value.replace(/[^\d-]/g, ""))} placeholder="не задан" className="max-w-48 font-mono" />
                {tg.last_start && tg.last_start.linked && String(tg.last_start.chat_id) !== chat && (
                  <Button variant="outline" onClick={() => setChat(String(tg.last_start!.chat_id))}>
                    Использовать чат {tg.last_start.name}
                  </Button>
                )}
              </div>
              <p className="text-xs text-muted-foreground">
                {tg.last_start
                  ? `Последний /start: ${tg.last_start.name} ${ago(tg.last_start.ts)}${tg.last_start.linked ? "" : " (аккаунт не привязан к пользователю панели)"}.`
                  : `Напишите боту /start со своего аккаунта, затем обновите страницу.`}
              </p>
            </div>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" disabled={!tg.chat_id} onClick={() => act(() => post("/alerts/telegram/test"), { success: "Отправлено" })}>
                <BellRing /> Тестовое сообщение
              </Button>
              <Button variant="outline" size="sm" disabled={!tg.chat_id} onClick={() => act(() => post("/alerts/summary"), { success: "Сводка отправлена" })}>
                Сводка сейчас
              </Button>
            </div>
          </CardContent>
        )}
      </Card>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Расписание</CardTitle>
          <CardDescription>Часовой пояс берётся из браузера (UTC{tz >= 0 ? "+" : "−"}{Math.abs(tz / 60)}).</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4 text-sm">
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div>Тихие часы</div>
              <div className="text-xs text-muted-foreground">Critical приходят всегда, остальное — после окончания.</div>
            </div>
            <div className="flex items-center gap-2">
              <HourInput value={prefs.quiet_from} onChange={(v) => set("quiet_from", v)} />–<HourInput value={prefs.quiet_to} onChange={(v) => set("quiet_to", v)} />
              <Switch checked={prefs.quiet_enabled} onCheckedChange={(v) => set("quiet_enabled", v)} />
            </div>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div>Утренняя сводка</div>
              <div className="text-xs text-muted-foreground">Активные алерты, uptime, бэкапы, обновления — в {hour(prefs.summary_hour)}.</div>
            </div>
            <div className="flex items-center gap-2">
              <HourInput value={prefs.summary_hour} onChange={(v) => set("summary_hour", v)} />
              <Switch checked={prefs.summary_enabled} onCheckedChange={(v) => set("summary_enabled", v)} />
            </div>
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <div>
              <div>Повтор активного алерта</div>
              <div className="text-xs text-muted-foreground">Если не ack и не mute. 0 — не повторять.</div>
            </div>
            <div className="flex items-center gap-2">
              <Input type="number" min={0} className="h-8 w-20" value={prefs.repeat_hours} onChange={(e) => set("repeat_hours", Math.max(0, Number(e.target.value)))} /> ч
            </div>
          </div>
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <HeartPulse className="size-3.5" /> Heartbeat во внешний мониторинг: {data.heartbeat ? "включён" : "выключен (STYK_HEARTBEAT_URL)"}
          </div>
          <Button size="sm" disabled={!dirty} onClick={save}>
            Сохранить
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}

export default function AlertsPage() {
  const { data, isLoading } = useQuery({ queryKey: ["alerts"], queryFn: () => api<AlertsResp>("/alerts"), refetchInterval: 20_000 });
  const [tab, setTab] = useState("active");
  if (isLoading || !data) return <Skeleton className="h-96" />;
  const critical = data.active.filter((e) => e.severity === "critical").length;
  return (
    <>
      <PageHeader
        title="Алерты"
        description={
          <span className="flex flex-wrap items-center gap-x-3">
            <span>{data.active.length ? `${data.active.length} активных${critical ? `, ${critical} critical` : ""}` : "активных нет"}</span>
            <span>{data.rules.filter((r) => r.enabled).length} из {data.rules.length} правил включено</span>
            <span>Telegram: {data.telegram.configured ? (data.telegram.chat_id ? "подключён" : "чат не выбран") : "не настроен"}</span>
          </span>
        }
      />
      {data.telegram.configured && !data.telegram.chat_id && (
        <Alert className="mb-4">
          <AlertTitle>Уведомления пока никуда не уходят</AlertTitle>
          <AlertDescription>Напишите боту @{data.telegram.bot_username} команду /start и выберите чат во вкладке «Уведомления».</AlertDescription>
        </Alert>
      )}
      <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
        <TabsList className="mb-4">
          <TabsTrigger value="active">
            Активные
            {data.active.length > 0 && <Badge variant={critical ? "destructive" : "secondary"}>{data.active.length}</Badge>}
          </TabsTrigger>
          <TabsTrigger value="history">История</TabsTrigger>
          <TabsTrigger value="rules">Правила</TabsTrigger>
          <TabsTrigger value="uptime">Uptime</TabsTrigger>
          <TabsTrigger value="notify">Уведомления</TabsTrigger>
        </TabsList>
        <TabsContent value="active">
          <Events events={data.active} active />
        </TabsContent>
        <TabsContent value="history">
          <Events events={data.history} />
        </TabsContent>
        <TabsContent value="rules">
          <Rules rules={data.rules} />
        </TabsContent>
        <TabsContent value="uptime">
          <Uptime />
        </TabsContent>
        <TabsContent value="notify">
          <Notifications key={JSON.stringify([data.prefs, data.telegram.chat_id])} data={data} />
        </TabsContent>
      </Tabs>
    </>
  );
}
