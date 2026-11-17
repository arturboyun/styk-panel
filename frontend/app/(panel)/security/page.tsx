"use client";

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { AlertTriangle, Ban, CheckCircle2, Copy, Download, KeyRound, Network, RefreshCw, ShieldAlert, ShieldCheck, Terminal } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { TimeChart } from "@/components/charts/time-chart";
import { useConfirm } from "@/components/confirm";
import { Empty, PageHeader } from "@/components/page";
import { api, post } from "@/lib/api";
import { ago } from "@/lib/format";
import { useAction } from "@/lib/ops";
import { AccessTab, useBlockIp } from "@/components/access-panel";
import type { Finding, FirewallTable, SecurityOverview, SshActivity } from "@/lib/types";
import { cn } from "@/lib/utils";

const LEVEL = {
  critical: { icon: ShieldAlert, ring: "border-destructive/40 bg-destructive/5", text: "text-destructive" },
  warning: { icon: AlertTriangle, ring: "border-warning/40 bg-warning/5", text: "text-warning" },
  ok: { icon: CheckCircle2, ring: "", text: "text-success" },
} as const;

function FindingCard({ f }: { f: Finding }) {
  const l = LEVEL[f.level];
  return (
    <div className={cn("rounded-xl border p-4", l.ring)}>
      <div className="flex items-start gap-3">
        <l.icon className={cn("mt-0.5 size-5 shrink-0", l.text)} />
        <div className="min-w-0 space-y-1">
          <div className="font-medium">{f.title}</div>
          <div className="text-sm text-muted-foreground">{f.detail}</div>
          {f.fix && (
            <div className="group relative mt-2 rounded-md bg-muted/70 p-2.5 pr-9 text-xs leading-relaxed">
              {f.fix}
              <button
                className="absolute top-2 right-2 text-muted-foreground opacity-0 transition group-hover:opacity-100 hover:text-foreground"
                onClick={() => navigator.clipboard.writeText(f.fix!).then(() => toast.success("Скопировано"))}
                aria-label="Скопировать"
              >
                <Copy className="size-3.5" />
              </button>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value, hint }: { label: string; value: React.ReactNode; hint?: React.ReactNode }) {
  return (
    <Card className="gap-1 py-4">
      <CardContent className="px-4">
        <div className="text-xs text-muted-foreground">{label}</div>
        <div className="mt-1 text-2xl font-semibold tabular">{value}</div>
        {hint && <div className="text-xs text-muted-foreground">{hint}</div>}
      </CardContent>
    </Card>
  );
}

function SshTab() {
  const block = useBlockIp();
  const [hours, setHours] = useState("24");
  const { data, isLoading, error } = useQuery({
    queryKey: ["ssh", hours],
    queryFn: () => api<SshActivity>(`/security/ssh?hours=${hours}`),
    refetchInterval: 60_000,
  });
  if (error) return <Empty title="Журнал SSH недоступен">{String(error.message)}</Empty>;
  if (isLoading || !data) return <Skeleton className="h-96" />;
  const maxUser = data.top_users[0]?.[1] ?? 1;
  const perHour = data.failed / Math.max(1, data.hours);
  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm text-muted-foreground">Неудачные попытки входа и успешные сессии из журнала sshd</div>
        <Select value={hours} onValueChange={(v) => setHours(String(v))} items={{ "24": "24 часа", "168": "7 дней", "720": "30 дней" }}>
          <SelectTrigger className="w-36">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="24">24 часа</SelectItem>
            <SelectItem value="168">7 дней</SelectItem>
            <SelectItem value="720">30 дней</SelectItem>
          </SelectContent>
        </Select>
      </div>
      <div className="grid gap-3 sm:grid-cols-3">
        <Stat label="Неудачных попыток" value={data.failed.toLocaleString("ru")} hint={`≈ ${Math.round(perHour)} в час`} />
        <Stat label="Уникальных IP" value={data.unique_ips.toLocaleString("ru")} />
        <Stat label="Успешных входов" value={data.accepted.length} hint={data.accepted[0] ? `последний ${ago(data.accepted[0].ts)}` : undefined} />
      </div>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Попытки по часам</CardTitle>
        </CardHeader>
        <CardContent>
          {data.hourly.length < 2 ? (
            <div className="py-10 text-center text-sm text-muted-foreground">Мало данных</div>
          ) : (
            <TimeChart
              x={data.hourly.map((h) => h[0])}
              series={[{ label: "Попытки", color: "--chart-5", values: data.hourly.map((h) => h[1]), fill: true }]}
              yFormat={(v) => v.toFixed(0)}
            />
          )}
        </CardContent>
      </Card>
      <div className="grid gap-4 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Топ IP</CardTitle>
          </CardHeader>
          <CardContent className="px-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-4">IP</TableHead>
                  <TableHead className="text-right">Попыток</TableHead>
                  <TableHead>Логин</TableHead>
                  <TableHead>Последняя</TableHead>
                  <TableHead className="w-10 pr-4" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.top_ips.slice(0, 20).map((ip) => (
                  <TableRow key={ip.ip}>
                    <TableCell className="pl-4 font-mono text-xs">{ip.ip}</TableCell>
                    <TableCell className="text-right tabular">{ip.count.toLocaleString("ru")}</TableCell>
                    <TableCell className="max-w-32 truncate font-mono text-xs">{ip.last_user}</TableCell>
                    <TableCell className="text-xs text-muted-foreground">{ago(ip.last)}</TableCell>
                    <TableCell className="pr-4">
                      <Button variant="ghost" size="icon-sm" aria-label="Заблокировать" title="Заблокировать" onClick={() => block(ip.ip, `SSH brute force: ${ip.count} попыток`)}>
                        <Ban />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Какие логины подбирают</CardTitle>
          </CardHeader>
          <CardContent className="space-y-1.5">
            {data.top_users.slice(0, 15).map(([u, n]) => (
              <div key={u} className="grid grid-cols-[8rem_minmax(0,1fr)_3.5rem] items-center gap-2 text-xs">
                <span className="truncate font-mono">{u}</span>
                <div className="h-1.5 overflow-hidden rounded-full bg-muted">
                  <div className="h-full rounded-full bg-chart-5" style={{ width: `${(n / maxUser) * 100}%` }} />
                </div>
                <span className="text-right tabular text-muted-foreground">{n.toLocaleString("ru")}</span>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Успешные входы</CardTitle>
          <CardDescription>Отпечаток ключа помогает отличить deploy-ключ GitHub Actions от личного</CardDescription>
        </CardHeader>
        <CardContent className="px-0">
          {data.accepted.length === 0 ? (
            <div className="px-4 text-sm text-muted-foreground">Нет входов за период</div>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-4">Когда</TableHead>
                  <TableHead>Пользователь</TableHead>
                  <TableHead>IP</TableHead>
                  <TableHead className="pr-4">Ключ</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.accepted.slice(0, 50).map((l, i) => (
                  <TableRow key={i}>
                    <TableCell className="pl-4 text-xs whitespace-nowrap">{new Date(l.ts * 1000).toLocaleString("ru")}</TableCell>
                    <TableCell className="font-mono text-xs">{l.user}</TableCell>
                    <TableCell className="font-mono text-xs">{l.ip}</TableCell>
                    <TableCell className="max-w-72 truncate pr-4 font-mono text-xs text-muted-foreground" title={l.key}>
                      {l.method === "publickey" ? l.key : l.method}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}

function PortsTab({ data }: { data: SecurityOverview }) {
  const ports = [...data.ports].sort((a, b) => Number(b.public) - Number(a.public) || a.port - b.port);
  return (
    <Card className="py-0">
      <CardContent className="px-0">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead className="pl-4">Порт</TableHead>
              <TableHead>Адрес</TableHead>
              <TableHead>Процесс</TableHead>
              <TableHead className="pr-4">Доступ</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {ports.map((p) => (
              <TableRow key={`${p.proto}${p.address}${p.port}`}>
                <TableCell className="pl-4 font-mono">
                  {p.port}
                  <span className="text-muted-foreground">/{p.proto}</span>
                </TableCell>
                <TableCell className="font-mono text-xs">{p.address}</TableCell>
                <TableCell className="text-sm">
                  {p.process || "—"}
                  {p.process === "docker-proxy" && (
                    <Badge variant="outline" className="ml-2 font-normal">
                      опубликован Docker
                    </Badge>
                  )}
                </TableCell>
                <TableCell className="pr-4">
                  {p.public ? (
                    <Badge variant={[22, 80, 443].includes(p.port) ? "secondary" : "destructive"}>снаружи</Badge>
                  ) : (
                    <span className="text-xs text-muted-foreground">только локально</span>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
    </Card>
  );
}

const RISKY = /^(docker|containerd|runc|systemd|linux-image|openssh|libc6)/;

function UpdatesTab({ data }: { data: SecurityOverview }) {
  const act = useAction();
  const confirm = useConfirm();
  const u = data.updates;
  if (!u) return <Empty title="Нет данных об обновлениях">{data.errors.updates}</Empty>;
  const risky = u.packages.filter((p) => RISKY.test(p.name)).map((p) => p.name);
  const sorted = [...u.packages].sort((a, b) => Number(b.security) - Number(a.security) || a.name.localeCompare(b.name));
  const install = async () => {
    const ok = await confirm({
      title: `Установить ${u.packages.length} обновлений?`,
      description: (
        <div className="space-y-2">
          <p>apt-get upgrade выполнится на сервере как отдельная systemd-задача и не прервётся, если закрыть страницу.</p>
          {risky.length > 0 && (
            <p className="rounded-md bg-warning/10 p-2 text-warning">
              Среди пакетов есть {risky.slice(0, 6).join(", ")}
              {risky.length > 6 ? "…" : ""}: обновление может перезапустить Docker, и все контейнеры (включая панель) поднимутся заново через несколько секунд.
            </p>
          )}
        </div>
      ),
      confirmText: "Установить",
    });
    if (ok) await act(() => post("/security/updates/apply"), { job: "Установка обновлений", invalidate: [["security"]] });
  };
  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <Badge variant={u.packages.some((p) => p.security) ? "destructive" : "secondary"}>
          security: {u.packages.filter((p) => p.security).length}
        </Badge>
        <span className="text-sm text-muted-foreground">
          всего {u.packages.length} пакетов{u.lists_updated ? `, списки обновлены ${ago(u.lists_updated)}` : ""}
        </span>
        {u.reboot_required && <Badge variant="outline" className="border-warning/50 text-warning">нужна перезагрузка: {u.reboot_packages.join(", ")}</Badge>}
        <Button className="ml-auto" disabled={u.packages.length === 0} onClick={install}>
          <Download /> Установить обновления
        </Button>
      </div>
      <Card className="py-0">
        <CardContent className="px-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="pl-4">Пакет</TableHead>
                <TableHead>Текущая</TableHead>
                <TableHead>Новая</TableHead>
                <TableHead className="pr-4">Репозиторий</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {sorted.map((p) => (
                <TableRow key={p.name}>
                  <TableCell className="pl-4 font-mono text-xs">
                    {p.name}
                    {p.security && <Badge variant="destructive" className="ml-2">security</Badge>}
                  </TableCell>
                  <TableCell className="max-w-48 truncate font-mono text-xs text-muted-foreground">{p.current}</TableCell>
                  <TableCell className="max-w-48 truncate font-mono text-xs">{p.candidate}</TableCell>
                  <TableCell className="pr-4 text-xs text-muted-foreground">{p.suite}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}

function FirewallTab() {
  const { data, isLoading, error } = useQuery({ queryKey: ["firewall"], queryFn: () => api<FirewallTable[]>("/security/firewall") });
  if (error) return <Empty title="nft недоступен">{error.message}</Empty>;
  if (isLoading || !data) return <Skeleton className="h-64" />;
  return (
    <div className="space-y-2">
      <p className="text-sm text-muted-foreground">Только просмотр. Таблицы docker/iptables-nft управляются Docker, правила Cloudflare — /etc/cf-origin.nft.</p>
      {data.map((t) => {
        const rules = t.body.split("\n").filter((l) => l.trim() && !/^\s*(chain|type|policy|})/.test(l)).length;
        return (
          <Collapsible key={`${t.family}${t.name}`} className="rounded-xl border">
            <CollapsibleTrigger className="flex w-full items-center gap-3 px-4 py-3 text-left hover:bg-muted/40">
              <Network className="size-4 text-muted-foreground" />
              <span className="font-mono text-sm">
                {t.family} {t.name}
              </span>
              <Badge variant="secondary" className="ml-auto">
                {rules} строк
              </Badge>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <pre className="max-h-[32rem] overflow-auto border-t bg-muted/30 p-4 font-mono text-xs leading-relaxed">{t.body}</pre>
            </CollapsibleContent>
          </Collapsible>
        );
      })}
    </div>
  );
}

function SshdTab({ data }: { data: SecurityOverview }) {
  if (data.errors.sshd) return <Empty title="sshd -T недоступен">{data.errors.sshd}</Empty>;
  const bad: Record<string, string> = { passwordauthentication: "yes", permitrootlogin: "yes", permitemptypasswords: "yes", kbdinteractiveauthentication: "yes" };
  return (
    <Card>
      <CardHeader>
        <CardTitle className="text-sm">Действующие настройки sshd</CardTitle>
        <CardDescription>Результат «sshd -T»: что реально применено с учётом sshd_config.d</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-x-8 sm:grid-cols-2">
        {Object.entries(data.sshd).map(([k, v]) => (
          <div key={k} className="flex justify-between border-b py-2 font-mono text-xs last:border-0">
            <span className="text-muted-foreground">{k}</span>
            <span className={cn(bad[k] === v && "font-semibold text-destructive")}>{v}</span>
          </div>
        ))}
      </CardContent>
    </Card>
  );
}

export default function SecurityPage() {
  const { data, isLoading, refetch, isFetching } = useQuery({
    queryKey: ["security"],
    queryFn: () => api<SecurityOverview>("/security/overview"),
    refetchInterval: 60_000,
  });
  const critical = data?.findings.filter((f) => f.level === "critical").length ?? 0;
  const warnings = data?.findings.filter((f) => f.level === "warning").length ?? 0;
  return (
    <>
      <PageHeader
        title="Безопасность"
        description="SSH, открытые порты, файрвол и обновления. Только рекомендации: панель сама ничего не меняет в системе, кроме установки обновлений по кнопке."
        actions={
          <Button variant="outline" size="sm" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={cn(isFetching && "animate-spin")} /> Обновить
          </Button>
        }
      />
      {isLoading || !data ? (
        <Skeleton className="h-64" />
      ) : (
        <>
          <div className="mb-3 flex items-center gap-2 text-sm">
            {critical + warnings === 0 ? (
              <span className="flex items-center gap-2 text-success">
                <ShieldCheck className="size-4" /> Проблем не найдено
              </span>
            ) : (
              <span className="text-muted-foreground">
                {critical > 0 && <span className="font-medium text-destructive">{critical} критичных</span>}
                {critical > 0 && warnings > 0 && " · "}
                {warnings > 0 && <span className="font-medium text-warning">{warnings} предупреждений</span>}
              </span>
            )}
          </div>
          <div className="mb-6 grid gap-3 lg:grid-cols-2">
            {data.findings.map((f) => (
              <FindingCard key={f.title} f={f} />
            ))}
          </div>
          <Tabs defaultValue="ssh">
            <TabsList className="mb-4">
              <TabsTrigger value="ssh">
                <Terminal /> SSH
              </TabsTrigger>
              <TabsTrigger value="ports">Порты</TabsTrigger>
              <TabsTrigger value="updates">
                Обновления
                {data.updates && data.updates.packages.length > 0 && (
                  <Badge variant="secondary" className="ml-1 h-4 px-1.5 text-[10px]">
                    {data.updates.packages.length}
                  </Badge>
                )}
              </TabsTrigger>
              <TabsTrigger value="access">
                <Ban /> Доступ
              </TabsTrigger>
              <TabsTrigger value="firewall">Файрвол</TabsTrigger>
              <TabsTrigger value="sshd">
                <KeyRound /> sshd
              </TabsTrigger>
            </TabsList>
            <TabsContent value="ssh">
              <SshTab />
            </TabsContent>
            <TabsContent value="ports">
              <PortsTab data={data} />
            </TabsContent>
            <TabsContent value="updates">
              <UpdatesTab data={data} />
            </TabsContent>
            <TabsContent value="access">
              <AccessTab />
            </TabsContent>
            <TabsContent value="firewall">
              <FirewallTab />
            </TabsContent>
            <TabsContent value="sshd">
              <SshdTab data={data} />
            </TabsContent>
          </Tabs>
        </>
      )}
    </>
  );
}
