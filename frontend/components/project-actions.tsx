"use client";

import Link from "next/link";
import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDownToLine, ChevronDown, Copy, KeyRound, MoreHorizontal, Play, Power, RefreshCw, RotateCw, Skull, Square, SquareTerminal, Trash2, Webhook } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { useConfirm } from "@/components/confirm";
import { api, post } from "@/lib/api";
import { ago, bytes } from "@/lib/format";
import { useAction } from "@/lib/ops";
import type { Container, DeployHook, DockerDiskUsage, Project } from "@/lib/types";

type ComposeAction = "up" | "pull" | "redeploy" | "restart" | "stop" | "down";

const ACTIONS: Record<ComposeAction, { label: string; verb: string; danger?: boolean; hint: string }> = {
  restart: { label: "Перезапустить", verb: "docker compose restart", hint: "Перезапуск контейнеров без пересоздания." },
  redeploy: { label: "Pull + redeploy", verb: "docker compose pull && up -d", hint: "Скачать новые образы и пересоздать изменившиеся контейнеры." },
  up: { label: "Поднять", verb: "docker compose up -d", hint: "Создать и запустить недостающие контейнеры." },
  pull: { label: "Скачать образы", verb: "docker compose pull", hint: "Только pull, без перезапуска." },
  stop: { label: "Остановить", verb: "docker compose stop", danger: true, hint: "Контейнеры остановятся, но не удалятся." },
  down: { label: "Down", verb: "docker compose down", danger: true, hint: "Контейнеры и сети проекта удалятся (тома останутся)." },
};

const isProxy = (s: string) => s.includes("caddy");

function ActionDialog({ project, action, onClose }: { project: Project; action: ComposeAction | null; onClose: () => void }) {
  const act = useAction();
  const services = [...new Set(project.containers.map((c) => c.service).filter((s): s is string => !!s))].sort();
  const [picked, setPicked] = useState<string[] | null>(null);
  const chosen = picked ?? (project.protected ? [] : services);
  const meta = action ? ACTIONS[action] : null;
  const all = chosen.length === services.length && !project.protected;
  const disabled = project.protected && chosen.length === 0;
  return (
    <Dialog open={!!action} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>
            {meta?.label}: {project.name}
          </DialogTitle>
          <DialogDescription render={<div />}>
            <p>{meta?.hint}</p>
            <code className="mt-1 block font-mono text-xs">{meta?.verb}</code>
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-2">
          <div className="text-sm font-medium">Сервисы</div>
          {project.protected && <p className="text-xs text-muted-foreground">Системный проект: выберите сервисы явно. Caddy не перезапускается — только reload конфигурации.</p>}
          <div className="grid grid-cols-2 gap-2">
            {services.map((s) => (
              <label key={s} className="flex items-center gap-2 text-sm">
                <Checkbox
                  checked={chosen.includes(s)}
                  disabled={project.protected && isProxy(s)}
                  onCheckedChange={(v) => setPicked(v ? [...chosen, s] : chosen.filter((x) => x !== s))}
                />
                <span className="font-mono">{s}</span>
              </label>
            ))}
          </div>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Отмена
          </Button>
          <Button
            variant={meta?.danger ? "destructive" : "default"}
            disabled={disabled}
            onClick={async () => {
              const r = await act(() => post(`/docker/projects/${encodeURIComponent(project.name)}/action`, { action, services: all ? [] : chosen }), {
                job: `${meta?.label}: ${project.name}`,
                invalidate: [["projects"]],
              });
              if (r) onClose();
            }}
          >
            {meta?.label}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function ProjectActions({ project }: { project: Project }) {
  const [action, setAction] = useState<ComposeAction | null>(null);
  const noDir = !project.working_dir;
  return (
    <>
      <Button size="sm" variant="outline" disabled={noDir} onClick={() => setAction("restart")}>
        <RotateCw /> Перезапустить
      </Button>
      <Button size="sm" disabled={noDir} onClick={() => setAction("redeploy")}>
        <ArrowDownToLine /> Pull + redeploy
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button size="sm" variant="outline" disabled={noDir} />}>
          Ещё <ChevronDown />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onClick={() => setAction("up")}>
            <Play /> Поднять (up -d)
          </DropdownMenuItem>
          <DropdownMenuItem onClick={() => setAction("pull")}>
            <ArrowDownToLine /> Только pull
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem variant="destructive" disabled={project.protected} onClick={() => setAction("stop")}>
            <Square /> Остановить
          </DropdownMenuItem>
          <DropdownMenuItem variant="destructive" disabled={project.protected} onClick={() => setAction("down")}>
            <Power /> Down
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <ActionDialog key={action ?? "none"} project={project} action={action} onClose={() => setAction(null)} />
    </>
  );
}

export function ContainerActions({ c, protectedProject }: { c: Pick<Container, "id" | "name" | "state" | "service">; protectedProject: boolean }) {
  const act = useAction();
  const confirm = useConfirm();
  const qc = useQueryClient();
  const run = async (action: "start" | "stop" | "restart" | "kill") => {
    if (action === "stop" || action === "kill") {
      const ok = await confirm({ title: `${action === "kill" ? "Убить" : "Остановить"} ${c.name}?`, destructive: true, confirmText: action === "kill" ? "SIGKILL" : "Остановить" });
      if (!ok) return;
    }
    await act(() => post(`/docker/containers/${c.id}/action`, { action }), { success: `${c.name}: ${action}` });
    qc.invalidateQueries({ queryKey: ["projects"] });
    qc.invalidateQueries({ queryKey: ["container", c.id] });
  };
  const running = c.state === "running";
  const proxy = protectedProject && isProxy(c.service ?? c.name);
  return (
    <DropdownMenu>
      <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Действия" />}>
        <MoreHorizontal />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {running ? (
          <DropdownMenuItem disabled={proxy} onClick={() => run("restart")}>
            <RefreshCw /> Перезапустить
          </DropdownMenuItem>
        ) : (
          <DropdownMenuItem onClick={() => run("start")}>
            <Play /> Запустить
          </DropdownMenuItem>
        )}
        {running && (
          <DropdownMenuItem render={<Link href={`/containers/view/?id=${c.id}&tab=terminal`} />}>
            <SquareTerminal /> Терминал
          </DropdownMenuItem>
        )}
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" disabled={!running || protectedProject} onClick={() => run("stop")}>
          <Square /> Остановить
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" disabled={!running || protectedProject} onClick={() => run("kill")}>
          <Skull /> Kill
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export function HookCard({ project }: { project: Project }) {
  const act = useAction();
  const confirm = useConfirm();
  const { data, refetch } = useQuery({ queryKey: ["hooks"], queryFn: () => api<DeployHook[]>("/docker/hooks") });
  const hook = data?.find((h) => h.project === project.name);
  const [secret, setSecret] = useState<string | null>(null);
  const url = typeof window !== "undefined" ? `${location.origin}/api/hooks/deploy/${project.name}` : "";
  const services = project.protected ? project.containers.map((c) => c.service).filter((s) => s && !isProxy(s)).slice(0, 1) : [];
  const body = JSON.stringify(services.length ? { services } : {});
  const curl = `BODY='${body}'
TS=$(date +%s)
SIG=$(printf '%s.%s' "$TS" "$BODY" | openssl dgst -sha256 -hmac "$STYK_HOOK_SECRET" -hex | sed 's/^.* //')
curl -fsS -X POST ${url} \\
  -H "X-Styk-Timestamp: $TS" -H "X-Styk-Signature: sha256=$SIG" \\
  -H 'content-type: application/json' -d "$BODY"`;
  const create = async () => {
    if (hook && !(await confirm({ title: "Перевыпустить секрет?", description: "Старый секрет перестанет работать сразу.", confirmText: "Перевыпустить" }))) return;
    const r = await act(() => post<{ secret: string }>(`/docker/projects/${encodeURIComponent(project.name)}/hook`));
    if (r) {
      setSecret(r.secret);
      refetch();
    }
  };
  const remove = async () => {
    if (!(await confirm({ title: "Удалить webhook?", destructive: true, confirmText: "Удалить" }))) return;
    await act(() => api(`/docker/projects/${encodeURIComponent(project.name)}/hook`, { method: "DELETE" }), { success: "Webhook удалён" });
    refetch();
  };
  const copy = (s: string) => navigator.clipboard.writeText(s).then(() => toast.success("Скопировано"));
  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-3">
        <div>
          <CardTitle className="flex items-center gap-2 text-sm">
            <Webhook className="size-4" /> Deploy webhook
          </CardTitle>
          <CardDescription>
            POST из CI запускает pull + up -d этого проекта. Подпись HMAC-SHA256 с меткой времени (±5 минут).
          </CardDescription>
        </div>
        <div className="flex gap-2">
          {hook && (
            <Button variant="ghost" size="sm" onClick={remove}>
              <Trash2 />
            </Button>
          )}
          <Button variant="outline" size="sm" onClick={create} disabled={!project.working_dir}>
            <KeyRound /> {hook ? "Перевыпустить" : "Создать"}
          </Button>
        </div>
      </CardHeader>
      <CardContent className="space-y-3 text-sm">
        {hook ? (
          <div className="text-muted-foreground">
            Создан {ago(hook.created_at)} ({hook.created_by}) · {hook.last_used_at ? `последний вызов ${ago(hook.last_used_at)}` : "ещё не вызывался"}
          </div>
        ) : (
          <div className="text-muted-foreground">Не настроен</div>
        )}
        {secret && (
          <div className="space-y-2 rounded-lg border border-warning/40 bg-warning/5 p-3">
            <div className="text-xs font-medium text-warning">Секрет показывается один раз — сохраните его в секретах CI как STYK_HOOK_SECRET</div>
            <div className="flex items-center gap-2">
              <code className="min-w-0 flex-1 truncate rounded bg-muted px-2 py-1 font-mono text-xs">{secret}</code>
              <Button variant="ghost" size="icon-sm" onClick={() => copy(secret)}>
                <Copy />
              </Button>
            </div>
          </div>
        )}
        {hook && (
          <div className="group relative">
            <pre className="overflow-auto rounded-lg bg-muted/60 p-3 font-mono text-[11px] leading-relaxed">{curl}</pre>
            <Button variant="ghost" size="icon-sm" className="absolute top-2 right-2" onClick={() => copy(curl)}>
              <Copy />
            </Button>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function PruneButton() {
  const act = useAction();
  const [open, setOpen] = useState(false);
  const [cache, setCache] = useState(true);
  const [images, setImages] = useState("dangling");
  const { data } = useQuery({ queryKey: ["df"], queryFn: () => api<DockerDiskUsage>("/docker/df"), enabled: open });
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <Trash2 /> Очистка Docker
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Очистка Docker</DialogTitle>
            <DialogDescription>Запущенные контейнеры и тома не затрагиваются.</DialogDescription>
          </DialogHeader>
          <div className="space-y-4 text-sm">
            <div className="grid grid-cols-2 gap-3 rounded-lg bg-muted/50 p-3">
              <div>
                <div className="text-xs text-muted-foreground">Build cache</div>
                <div className="font-medium tabular">{bytes(data?.build_cache_reclaimable)}</div>
                <div className="text-xs text-muted-foreground">из {bytes(data?.build_cache_total)}</div>
              </div>
              <div>
                <div className="text-xs text-muted-foreground">Неиспользуемые образы</div>
                <div className="font-medium tabular">{bytes(data?.images_reclaimable)}</div>
                <div className="text-xs text-muted-foreground">
                  всего {data?.images_count ?? "—"} · {bytes(data?.images_total)}
                </div>
              </div>
            </div>
            <label className="flex items-center gap-2">
              <Checkbox checked={cache} onCheckedChange={(v) => setCache(!!v)} /> Удалить build cache
            </label>
            <div className="flex items-center justify-between gap-3">
              <span>Образы</span>
              <Select value={images} onValueChange={(v) => setImages(String(v))} items={{ none: "не трогать", dangling: "только dangling", unused: "все неиспользуемые" }}>
                <SelectTrigger className="w-52">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="none">не трогать</SelectItem>
                  <SelectItem value="dangling">только dangling</SelectItem>
                  <SelectItem value="unused">все неиспользуемые</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {images === "unused" && <p className="text-xs text-warning">Образы остановленных проектов тоже удалятся: при следующем up их придётся скачать заново.</p>}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setOpen(false)}>
              Отмена
            </Button>
            <Button
              variant="destructive"
              disabled={!cache && images === "none"}
              onClick={async () => {
                const r = await act(() => post("/docker/prune", { build_cache: cache, images }), { job: "Очистка Docker", invalidate: [["df"], ["overview"]] });
                if (r) setOpen(false);
              }}
            >
              Очистить
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}
