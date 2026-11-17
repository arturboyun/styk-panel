"use client";

import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban, Lock, Plus, Save, Trash2, Undo2, Workflow } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { useConfirm } from "@/components/confirm";
import { api, post } from "@/lib/api";
import { ago } from "@/lib/format";
import { Cancelled, errorMessage, useOps } from "@/lib/ops";
import type { AccessConfig, AccessEntry, AccessState } from "@/lib/types";
import { toast } from "sonner";

const KEY = ["access"];

const v4 = (ip: string) => {
  const p = ip.split(".").map(Number);
  return p.length === 4 && p.every((n) => Number.isInteger(n) && n >= 0 && n < 256) ? ((p[0] << 24) | (p[1] << 16) | (p[2] << 8) | p[3]) >>> 0 : null;
};

/** Whether `cidr` contains `ip` (IPv4 networks; IPv6 by exact match). */
function covers(cidr: string, ip: string): boolean {
  const [net, bits] = cidr.split("/");
  const a = v4(net);
  const b = v4(ip);
  if (a == null || b == null) return net === ip;
  const n = bits == null ? 32 : Number(bits);
  const mask = n === 0 ? 0 : (~0 << (32 - n)) >>> 0;
  return ((a & mask) >>> 0) === ((b & mask) >>> 0);
}

/** Saves the config; on the "no current SSH session would stay allowed" refusal asks to force. */
function useSaveAccess() {
  const { sudo } = useOps();
  const confirm = useConfirm();
  const qc = useQueryClient();
  return async (config: AccessConfig): Promise<boolean> => {
    const send = (force: boolean) => sudo(() => post<AccessState>("/security/access", { config, force }));
    try {
      let res: AccessState;
      try {
        res = await send(false);
      } catch (e) {
        const msg = errorMessage(e);
        if (!msg.includes("current SSH sessions")) throw e;
        const ok = await confirm({
          title: "Текущие SSH-сессии не попадают в список",
          description: `${msg}. Открытые сессии не оборвутся, но новое подключение с этих адресов будет отклонено. Применить всё равно?`,
          destructive: true,
          confirmText: "Применить",
        });
        if (!ok) return false;
        res = await send(true);
      }
      qc.setQueryData(KEY, res);
      qc.invalidateQueries({ queryKey: ["security"] });
      toast.success("Правила применены");
      return true;
    } catch (e) {
      if (!(e instanceof Cancelled)) toast.error(errorMessage(e));
      return false;
    }
  };
}

/** One-click block from other pages (e.g. the SSH brute-force list). */
export function useBlockIp() {
  const save = useSaveAccess();
  const confirm = useConfirm();
  return async (ip: string, comment: string) => {
    const ok = await confirm({ title: `Заблокировать ${ip}?`, description: "Все порты, включая сайты. Снять блокировку можно на вкладке «Доступ».", destructive: true, confirmText: "Заблокировать" });
    if (!ok) return;
    let cur: AccessState;
    try {
      cur = await api<AccessState>("/security/access");
    } catch (e) {
      toast.error(errorMessage(e));
      return;
    }
    if (cur.config.block.some((b) => b.cidr === ip)) return toast.info(`${ip} уже заблокирован`);
    await save({ ...cur.config, block: [...cur.config.block, { cidr: ip, comment }] });
  };
}

function EntryList({ items, onChange, placeholder }: { items: AccessEntry[]; onChange: (v: AccessEntry[]) => void; placeholder: string }) {
  const [cidr, setCidr] = useState("");
  const [comment, setComment] = useState("");
  const add = () => {
    const c = cidr.trim();
    if (!c || items.some((i) => i.cidr === c)) return;
    onChange([...items, { cidr: c, comment: comment.trim() }]);
    setCidr("");
    setComment("");
  };
  return (
    <div className="space-y-2">
      {items.length === 0 && <div className="text-sm text-muted-foreground">Пусто</div>}
      {items.map((e, i) => (
        <div key={e.cidr} className="flex items-center gap-2">
          <code className="w-44 shrink-0 truncate font-mono text-sm">{e.cidr}</code>
          <Input value={e.comment} onChange={(ev) => onChange(items.map((x, j) => (j === i ? { ...x, comment: ev.target.value } : x)))} placeholder="комментарий" className="h-8 text-xs" />
          <Button variant="ghost" size="icon-sm" aria-label="Удалить" onClick={() => onChange(items.filter((_, j) => j !== i))}>
            <Trash2 />
          </Button>
        </div>
      ))}
      <form
        className="flex items-center gap-2 pt-1"
        onSubmit={(e) => {
          e.preventDefault();
          add();
        }}
      >
        <Input value={cidr} onChange={(e) => setCidr(e.target.value)} placeholder={placeholder} className="h-8 w-44 shrink-0 font-mono text-sm" />
        <Input value={comment} onChange={(e) => setComment(e.target.value)} placeholder="комментарий" className="h-8 text-xs" />
        <Button type="submit" variant="outline" size="icon-sm" aria-label="Добавить" disabled={!cidr.trim()}>
          <Plus />
        </Button>
      </form>
    </div>
  );
}

function Editor({ state }: { state: AccessState }) {
  const save = useSaveAccess();
  const confirm = useConfirm();
  const [cfg, setCfg] = useState<AccessConfig>(state.config);
  const [busy, setBusy] = useState(false);
  const dirty = JSON.stringify(cfg) !== JSON.stringify(state.config);
  const allowed = (ip: string) => cfg.ssh_allow.some((a) => covers(a.cidr, ip));
  const outside = state.ssh_sessions.filter((s) => !allowed(s));

  const apply = async () => {
    const lines = [
      cfg.ssh_allow_enabled ? `SSH: только ${cfg.ssh_allow.length} адрес(ов) из списка${cfg.ssh_allow_github ? " и GitHub Actions" : ""}` : "SSH: открыт для всех",
      `Блокировка: ${cfg.block.length} адрес(ов) на всех портах`,
    ];
    if (!(await confirm({ title: "Применить правила доступа?", description: lines.join(". ") + ".", confirmText: "Применить" }))) return;
    setBusy(true);
    await save(cfg);
    setBusy(false);
  };

  return (
    <div className="space-y-4">
      <div className="grid gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader className="flex flex-row items-start justify-between gap-3">
            <div>
              <CardTitle className="flex items-center gap-2 text-sm">
                <Lock className="size-4" /> SSH только с этих адресов
              </CardTitle>
              <CardDescription>
                Остальным порт 22 закрыт. Уже открытые сессии не обрываются. Панель работает через HTTPS, поэтому даже при ошибке в списке её можно открыть и всё исправить.
              </CardDescription>
            </div>
            <Switch checked={cfg.ssh_allow_enabled} onCheckedChange={(v) => setCfg({ ...cfg, ssh_allow_enabled: v })} />
          </CardHeader>
          <CardContent className="space-y-3">
            <EntryList items={cfg.ssh_allow} onChange={(v) => setCfg({ ...cfg, ssh_allow: v })} placeholder="1.2.3.4 или 10.0.0.0/24" />
            <label className="flex items-start justify-between gap-3 border-t pt-3">
              <span className="text-sm">
                <span className="flex items-center gap-2 font-medium">
                  <Workflow className="size-4" /> Разрешить GitHub Actions
                </span>
                <span className="block text-xs text-muted-foreground">
                  Для деплоя из CI по SSH. Официальные диапазоны из api.github.com/meta, обновляются раз в сутки
                  {state.github_ranges > 0 ? ` (${state.github_ranges.toLocaleString("ru")} сетей, ${ago(state.github_updated!)})` : ""}. Это все раннеры GitHub, не только ваши: вход
                  по-прежнему только по ключу.
                </span>
                {cfg.ssh_allow_github && state.github_ranges === 0 && (
                  <span className="block text-xs text-destructive">Список диапазонов ещё не загружен: запустите styk-access-github-refresh на сервере.</span>
                )}
              </span>
              <Switch checked={cfg.ssh_allow_github} onCheckedChange={(v) => setCfg({ ...cfg, ssh_allow_github: v })} />
            </label>
            <div className="flex flex-wrap items-center gap-2 border-t pt-3 text-xs text-muted-foreground">
              <span>Сейчас подключены:</span>
              {state.ssh_sessions.length === 0 && <span>никого</span>}
              {state.ssh_sessions.map((s) => (
                <Badge key={s} variant={allowed(s) ? "secondary" : "outline"} className="font-mono">
                  {s}
                  {!allowed(s) && (
                    <button type="button" className="ml-1 underline" onClick={() => setCfg({ ...cfg, ssh_allow: [...cfg.ssh_allow, { cidr: s, comment: "" }] })}>
                      добавить
                    </button>
                  )}
                </Badge>
              ))}
            </div>
            {state.config.ssh_allow_enabled && <div className="text-xs text-muted-foreground">Отклонено пакетов: {state.ssh_dropped.toLocaleString("ru")}</div>}
          </CardContent>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-sm">
              <Ban className="size-4" /> Заблокированные адреса
            </CardTitle>
            <CardDescription>
              Отбрасываются на всех портах, включая опубликованные Docker. Адреса Cloudflare, адреса из SSH-списка и ваш текущий адрес заблокировать нельзя.
            </CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <EntryList items={cfg.block} onChange={(v) => setCfg({ ...cfg, block: v })} placeholder="45.1.2.3 или 45.1.2.0/24" />
            <div className="border-t pt-3 text-xs text-muted-foreground">Отброшено пакетов: {state.block_dropped.toLocaleString("ru")}</div>
          </CardContent>
        </Card>
      </div>
      {cfg.ssh_allow_enabled && outside.length > 0 && (
        <Alert variant="destructive">
          <AlertTitle>Не все текущие подключения в списке</AlertTitle>
          <AlertDescription>{outside.join(", ")}: новое SSH-подключение с этих адресов будет отклонено (например, CI-деплой через SSH).</AlertDescription>
        </Alert>
      )}
      <div className="flex justify-end gap-2">
        <Button variant="outline" disabled={!dirty} onClick={() => setCfg(state.config)}>
          <Undo2 /> Отменить
        </Button>
        <Button disabled={!dirty || busy} onClick={apply}>
          <Save /> {busy ? "Применяю…" : "Применить"}
        </Button>
      </div>
    </div>
  );
}

export function AccessTab() {
  const { data, error } = useQuery({ queryKey: KEY, queryFn: () => api<AccessState>("/security/access"), refetchInterval: 30_000 });
  if (error) return <div className="text-sm text-destructive">{errorMessage(error)}</div>;
  if (!data) return <Skeleton className="h-80" />;
  return <Editor key={JSON.stringify(data.config)} state={data} />;
}
