"use client";

import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { CheckCircle2, Cloud, ExternalLink, Globe, Lock, Plus, Save, ShieldCheck, Trash2, XCircle } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { CodeEditor, DiffView } from "@/components/code-editor-lazy";
import { changedLines } from "@/lib/diff";
import { useConfirm } from "@/components/confirm";
import { Empty, PageHeader } from "@/components/page";
import { ContainerState, Dot } from "@/components/status";
import { api, post } from "@/lib/api";
import { ago, secondsSince } from "@/lib/format";
import { useAction } from "@/lib/ops";
import type { ApplyResult, CertInfo, DomainRow, DomainsResp } from "@/lib/types";

const isLocalCa = (c: CertInfo) => c.issuer.includes("Caddy local");

const TLS: Record<string, string> = { http: "HTTP-01", dns: "DNS (Cloudflare)", on_demand: "on-demand", internal: "внутренний" };

function CertBadge({ certs }: { certs: CertInfo[] }) {
  if (!certs.length) return <span className="text-xs text-muted-foreground">—</span>;
  const soonest = Math.min(...certs.map((c) => c.not_after));
  const days = Math.floor(-secondsSince(soonest) / 86_400);
  if (certs.every(isLocalCa))
    return (
      <Badge variant="outline" title="Caddy local CA: короткоживущий, продлевается автоматически">
        <ShieldCheck /> local CA
      </Badge>
    );
  const tone = days < 7 ? "destructive" : days < 14 ? "secondary" : "outline";
  return (
    <Badge variant={tone} className={days >= 7 && days < 14 ? "text-warning" : undefined} title={certs.map((c) => `${c.domain}: ${c.issuer}`).join("\n")}>
      <ShieldCheck /> {days} д
    </Badge>
  );
}

function ResultDialog({ result, onClose }: { result: ApplyResult | null; onClose: () => void }) {
  return (
    <Dialog open={!!result} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            {result?.ok ? <CheckCircle2 className="size-5 text-success" /> : <XCircle className="size-5 text-destructive" />}
            {result?.ok ? "Применено" : "Не применено"}
          </DialogTitle>
          <DialogDescription>{result?.ok ? "Caddy перечитал конфигурацию без перезапуска." : "Caddyfile остался прежним, сайты не затронуты."}</DialogDescription>
        </DialogHeader>
        <ol className="space-y-1 text-sm">
          {result?.steps.map((s, i) => (
            <li key={i} className="flex gap-2">
              <span className="text-muted-foreground tabular">{i + 1}.</span>
              <span>{s}</span>
            </li>
          ))}
        </ol>
        {result?.error && <pre className="max-h-60 overflow-auto rounded-md bg-destructive/10 p-3 font-mono text-xs whitespace-pre-wrap text-destructive">{result.error}</pre>}
        <DialogFooter>
          <Button onClick={onClose}>Закрыть</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface Upstreams {
  networks: string[];
  containers: { container: string; project: string | null; ports: number[]; names: string[] }[];
}

function AddSite({ open, onClose, onResult, cfDns }: { open: boolean; onClose: () => void; onResult: (r: ApplyResult) => void; cfDns: boolean }) {
  const act = useAction();
  const [f, setF] = useState({ domain: "", upstream: "", cloudflare_only: true, dns_challenge: true, compress: true, hsts: true });
  const [block, setBlock] = useState("");
  const [busy, setBusy] = useState(false);
  const { data: ups } = useQuery({ queryKey: ["domains", "upstreams"], queryFn: () => api<Upstreams>("/domains/upstreams"), enabled: open });
  const suggestions = useMemo(
    () => (ups?.containers ?? []).flatMap((c) => (c.ports.length ? c.ports : [80]).map((p) => ({ value: `${c.names[0] ?? c.container}:${p}`, project: c.project }))),
    [ups],
  );
  const ready = open && !!f.domain && !!f.upstream;
  useEffect(() => {
    if (!ready) return;
    const t = setTimeout(() => post<{ block: string }>("/domains/preview", f).then((r) => setBlock(r.block), () => setBlock("")), 250);
    return () => clearTimeout(t);
  }, [f, ready]);
  const shown = ready ? block : "";
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((s) => ({ ...s, [k]: v }));
  const opt = (k: "cloudflare_only" | "dns_challenge" | "compress" | "hsts", title: string, hint: string) => (
    <label className="flex items-start justify-between gap-3 py-1.5">
      <span>
        <span className="text-sm">{title}</span>
        <span className="block text-xs text-muted-foreground">{hint}</span>
      </span>
      <Switch checked={f[k]} onCheckedChange={(v) => set(k, v)} />
    </label>
  );
  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>Новый сайт</DialogTitle>
          <DialogDescription>Блок добавится в конец Caddyfile между маркерами styk-panel. Перед reload конфигурация проверяется через caddy validate.</DialogDescription>
        </DialogHeader>
        <div className="grid gap-5 md:grid-cols-2">
          <div className="space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="d">Домен</Label>
              <Input id="d" value={f.domain} onChange={(e) => set("domain", e.target.value)} placeholder="app.example.com" autoFocus />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="u">Upstream</Label>
              <Input id="u" value={f.upstream} onChange={(e) => set("upstream", e.target.value)} placeholder="container:8080" list="upstreams" className="font-mono" />
              <datalist id="upstreams">
                {suggestions.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.project ?? ""}
                  </option>
                ))}
              </datalist>
              <p className="text-xs text-muted-foreground">
                Контейнер должен быть в одной сети с Caddy{ups?.networks.length ? ` (${ups.networks.join(", ")})` : ""}.
              </p>
            </div>
            <div className="divide-y">
              {opt("cloudflare_only", "Только через Cloudflare", "import cloudflare_only — прямые запросы к IP отклоняются")}
              {opt("dns_challenge", "Сертификат через DNS Cloudflare", "работает за прокси Cloudflare и до появления DNS-записи")}
              {opt("compress", "Сжатие", "encode zstd gzip")}
              {opt("hsts", "HSTS", "Strict-Transport-Security на год")}
            </div>
            {cfDns ? (
              <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <Cloud className="size-3.5" /> A-запись в Cloudflare создастся автоматически (proxied).
              </p>
            ) : (
              <p className="text-xs text-muted-foreground">DNS-запись создайте вручную: CLOUDFLARE_API_TOKEN/STYK_PUBLIC_IP не заданы.</p>
            )}
          </div>
          <div className="space-y-1.5">
            <Label>Предпросмотр</Label>
            <pre className="h-full min-h-48 overflow-auto rounded-lg bg-muted/60 p-3 font-mono text-xs leading-relaxed">{shown || "Заполните домен и upstream"}</pre>
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Отмена
          </Button>
          <Button
            disabled={!shown || busy}
            onClick={async () => {
              setBusy(true);
              const r = await act(() => post<ApplyResult>("/domains", f), { invalidate: [["domains"]] });
              setBusy(false);
              if (r) {
                onResult(r);
                if (r.ok) {
                  onClose();
                  setF((s) => ({ ...s, domain: "", upstream: "" }));
                }
              }
            }}
          >
            <Plus /> {busy ? "Применяю…" : "Добавить"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SitesTable({ rows, onRemove }: { rows: DomainRow[]; onRemove: (d: string) => void }) {
  return (
    <Card className="py-0">
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="pl-4">Домен</TableHead>
            <TableHead>Upstream</TableHead>
            <TableHead className="hidden lg:table-cell">TLS</TableHead>
            <TableHead>Сертификат</TableHead>
            <TableHead className="hidden md:table-cell">Uptime</TableHead>
            <TableHead className="w-10 pr-4" />
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => {
            const up = r.uptime[0];
            return (
              <TableRow key={r.site.line}>
                <TableCell className="pl-4">
                  <div className="flex flex-col gap-0.5">
                    {r.site.hosts.map((h) => (
                      <a key={h} href={`https://${h.replace(/^\*\./, "")}`} target="_blank" rel="noreferrer" className="group inline-flex items-center gap-1 font-medium hover:underline">
                        {h}
                        <ExternalLink className="size-3 opacity-0 group-hover:opacity-60" />
                      </a>
                    ))}
                    <div className="flex flex-wrap gap-1">
                      {r.site.managed ? <Badge variant="secondary">панель</Badge> : <Badge variant="outline" className="text-muted-foreground"><Lock /> вручную</Badge>}
                      {r.site.cloudflare_only && (
                        <Badge variant="outline">
                          <Cloud /> CF only
                        </Badge>
                      )}
                    </div>
                  </div>
                </TableCell>
                <TableCell>
                  {r.site.redirect && <div className="text-xs text-muted-foreground">→ {r.site.redirect}</div>}
                  {r.site.static_files && <div className="text-xs text-muted-foreground">статические файлы</div>}
                  {r.upstreams.map((u) => (
                    <div key={u.address} className="flex items-center gap-2 text-xs">
                      <code className="font-mono">{u.address}</code>
                      {u.matches != null && u.matches > 1 ? (
                        <Badge variant="destructive">совпадает у {u.matches} контейнеров</Badge>
                      ) : u.state ? (
                        <ContainerState c={{ state: u.state as "running", health: (u.health as "healthy" | null) ?? null }} />
                      ) : (
                        <span className="text-muted-foreground">контейнер не найден</span>
                      )}
                    </div>
                  ))}
                  {r.upstreams[0]?.project && (
                    <Link href={`/projects/view/?name=${encodeURIComponent(r.upstreams[0].project)}`} className="text-xs text-muted-foreground hover:underline">
                      {r.upstreams[0].project}
                    </Link>
                  )}
                </TableCell>
                <TableCell className="hidden text-xs lg:table-cell">{TLS[r.site.tls] ?? r.site.tls}</TableCell>
                <TableCell>
                  <CertBadge certs={r.certs} />
                </TableCell>
                <TableCell className="hidden md:table-cell">
                  {up ? (
                    <span className="inline-flex items-center gap-1.5 text-xs">
                      <Dot tone={up.up == null ? "idle" : up.up ? "ok" : "bad"} />
                      {up.status ?? (up.up === false ? "нет ответа" : "—")}
                      {up.status != null && up.latency_ms != null && <span className="text-muted-foreground">{up.latency_ms} мс</span>}
                    </span>
                  ) : (
                    <span className="text-xs text-muted-foreground">—</span>
                  )}
                </TableCell>
                <TableCell className="pr-4">
                  {r.site.managed && (
                    <Button variant="ghost" size="icon-sm" aria-label="Удалить" onClick={() => onRemove(r.site.hosts[0])}>
                      <Trash2 />
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </Card>
  );
}

function RawEditor({ onResult }: { onResult: (r: ApplyResult) => void }) {
  const act = useAction();
  const qc = useQueryClient();
  const { data, refetch } = useQuery({ queryKey: ["domains", "caddyfile"], queryFn: () => api<{ path: string; content: string; mtime: number }>("/domains/caddyfile") });
  const [text, setText] = useState<string | null>(null);
  const [review, setReview] = useState(false);
  if (!data) return <Skeleton className="h-96" />;
  const value = text ?? data.content;
  const dirty = text != null && text !== data.content;
  return (
    <div className="space-y-3">
      <Alert>
        <AlertTitle>Общий Caddyfile всех сайтов</AlertTitle>
        <AlertDescription>
          Перед сохранением показывается diff, затем caddy validate → запись → reload. Если Caddy не примет конфигурацию, файл вернётся к прежней версии. Прежняя версия сохраняется в истории файла.
        </AlertDescription>
      </Alert>
      <div className="flex items-center justify-between gap-3">
        <code className="truncate text-xs text-muted-foreground">{data.path}</code>
        <div className="flex gap-2">
          <Button variant="ghost" size="sm" render={<Link href={`/files/edit/?path=${encodeURIComponent(data.path)}`} />}>
            История версий
          </Button>
          <Button size="sm" disabled={!dirty} onClick={() => setReview(true)}>
            <Save /> Сохранить
          </Button>
        </div>
      </div>
      <CodeEditor value={value} onChange={setText} path="Caddyfile" />
      <Dialog open={review} onOpenChange={setReview}>
        <DialogContent className="sm:max-w-5xl">
          <DialogHeader>
            <DialogTitle>
              Изменения: <span className="text-success">+{changedLines(data.content, value).added}</span> <span className="text-destructive">−{changedLines(data.content, value).removed}</span>
            </DialogTitle>
          </DialogHeader>
          <div className="max-h-[60vh] overflow-auto rounded-lg border">
            <DiffView before={data.content} after={value} path="Caddyfile" />
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setReview(false)}>
              Назад
            </Button>
            <Button
              onClick={async () => {
                const r = await act(() => post<ApplyResult>("/domains/caddyfile", { content: value, mtime: data.mtime }));
                setReview(false);
                if (r) {
                  onResult(r);
                  if (r.ok) {
                    setText(null);
                    await refetch();
                    qc.invalidateQueries({ queryKey: ["domains"] });
                  }
                }
              }}
            >
              Проверить и применить
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export default function DomainsPage() {
  const act = useAction();
  const confirm = useConfirm();
  const { data, isLoading } = useQuery({ queryKey: ["domains"], queryFn: () => api<DomainsResp>("/domains"), refetchInterval: 60_000 });
  const [adding, setAdding] = useState(false);
  const [result, setResult] = useState<ApplyResult | null>(null);
  const [tab, setTab] = useState("sites");

  const remove = async (domain: string) => {
    const ok = await confirm({ title: `Удалить сайт ${domain}?`, description: "Блок будет удалён из Caddyfile, Caddy перечитает конфигурацию. DNS-запись останется.", destructive: true, confirmText: "Удалить", typeToConfirm: domain });
    if (!ok) return;
    const r = await act(() => api<ApplyResult>(`/domains/site/${encodeURIComponent(domain)}`, { method: "DELETE" }), { invalidate: [["domains"]] });
    if (r) setResult(r);
  };

  const problems = data?.problems ?? [];
  const expiring = (data?.certs ?? []).filter((c) => !isLocalCa(c) && -secondsSince(c.not_after) < 14 * 86_400);

  return (
    <>
      <PageHeader
        title="Домены"
        description={
          data && (
            <span className="flex flex-wrap items-center gap-x-3">
              <span>{data.sites.length} сайтов</span>
              <span className="inline-flex items-center gap-1.5">
                <Dot tone={data.caddy.state === "running" ? "ok" : "bad"} /> {data.caddy.container}
              </span>
              <span>изменён {ago(data.mtime)}</span>
            </span>
          )
        }
        actions={
          <Button size="sm" onClick={() => setAdding(true)}>
            <Plus /> Новый сайт
          </Button>
        }
      />
      {problems.length > 0 && (
        <Alert variant="destructive" className="mb-4">
          <AlertTitle>{problems.some((p) => p.kind === "stale") ? "Caddy не видит изменения Caddyfile" : "Проблемы в общей сети Caddy"}</AlertTitle>
          <AlertDescription>
            <ul className="list-disc space-y-1 pl-4">
              {problems.map((p) => (
                <li key={p.kind + p.subject}>{p.message}</li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      )}
      {expiring.length > 0 && (
        <Alert variant="destructive" className="mb-4">
          <AlertTitle>Сертификаты скоро истекают</AlertTitle>
          <AlertDescription>{expiring.map((c) => c.domain).join(", ")}: Caddy обычно продлевает их за 30 дней, значит продление не проходит. Проверьте логи Caddy.</AlertDescription>
        </Alert>
      )}
      <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
        <TabsList className="mb-4">
          <TabsTrigger value="sites">Сайты</TabsTrigger>
          <TabsTrigger value="caddyfile">Caddyfile</TabsTrigger>
        </TabsList>
        <TabsContent value="sites">
          {isLoading ? (
            <Skeleton className="h-96" />
          ) : !data?.sites.length ? (
            <Empty icon={<Globe />} title="Сайтов нет">
              Caddyfile пуст или не найден.
            </Empty>
          ) : (
            <SitesTable rows={data.sites} onRemove={remove} />
          )}
          <p className="mt-3 text-xs text-muted-foreground">
            Панель меняет только блоки, которые сама добавила (между маркерами <code># &gt;&gt;&gt; styk-panel</code>). Рукописные блоки правятся во вкладке Caddyfile.
          </p>
        </TabsContent>
        <TabsContent value="caddyfile">
          <RawEditor onResult={setResult} />
        </TabsContent>
      </Tabs>
      <AddSite open={adding} onClose={() => setAdding(false)} onResult={setResult} cfDns={!!data?.cloudflare_dns} />
      <ResultDialog result={result} onClose={() => setResult(null)} />
    </>
  );
}
