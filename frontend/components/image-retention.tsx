"use client";

import { useState } from "react";
import { keepPreviousData, useQuery, useQueryClient } from "@tanstack/react-query";
import { Layers, Trash2 } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { Switch } from "@/components/ui/switch";
import { useConfirm } from "@/components/confirm";
import { api, post } from "@/lib/api";
import { ago, bytes } from "@/lib/format";
import { useAction } from "@/lib/ops";
import type { RetentionPolicy, RetentionResp } from "@/lib/types";

const REASON: Record<string, string> = { in_use: "используется", protected: "защищён", newest: "новый" };

function Editor({ data, onClose }: { data: RetentionResp; onClose: () => void }) {
  const act = useAction();
  const confirm = useConfirm();
  const qc = useQueryClient();
  const [p, setP] = useState<RetentionPolicy>(data.policy);
  const [tags, setTags] = useState(data.policy.protected_tags.join(", "));
  const body = { ...p, protected_tags: tags.split(",").map((t) => t.trim()).filter(Boolean) };
  const { data: preview, isFetching } = useQuery({
    queryKey: ["retention", p.keep, tags],
    queryFn: () => api<RetentionResp>(`/docker/images/retention?keep=${p.keep}&protected=${encodeURIComponent(tags)}`),
    placeholderData: keepPreviousData,
  });
  const plan = (preview ?? data).plan;
  const repos = plan.repos.filter((r) => r.remove.length > 0);
  const dirty = JSON.stringify(body) !== JSON.stringify(data.policy);
  const last = data.last;

  const save = () => act(() => post("/docker/images/retention", body), { success: "Правило сохранено", invalidate: [["retention"]] });
  const run = async () => {
    const ok = await confirm({
      title: `Удалить ${plan.remove_count} образов?`,
      description: `Освободится до ${bytes(plan.reclaim)}. Используемые контейнерами и защищённые теги не трогаются.`,
      destructive: true,
      confirmText: "Удалить",
    });
    if (!ok) return;
    const r = await act(() => post<RetentionResp["last"]>("/docker/images/retention/run", body));
    if (r) {
      qc.invalidateQueries({ queryKey: ["retention"] });
      qc.invalidateQueries({ queryKey: ["df"] });
    }
  };

  return (
    <>
      <div className="grid gap-4 sm:grid-cols-[8rem_1fr]">
        <div className="space-y-1.5">
          <Label htmlFor="keep">Хранить N</Label>
          <Input id="keep" type="number" min={1} max={50} value={p.keep} onChange={(e) => setP({ ...p, keep: Math.max(1, Math.min(50, Number(e.target.value) || 1)) })} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="tags">Защищённые теги</Label>
          <Input id="tags" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="rollback, latest" className="font-mono" />
        </div>
      </div>
      <label className="flex items-center justify-between gap-3 rounded-lg border p-3 text-sm">
        <span>
          <span className="block">Каждый день в {data.schedule}</span>
          <span className="text-xs text-muted-foreground">
            {last ? `Последний запуск ${ago(last.ts)} (${last.trigger === "timer" ? "по таймеру" : "вручную"}): удалено ${last.removed}, освобождено ${bytes(last.freed)}${last.errors.length ? `, ошибок ${last.errors.length}` : ""}` : "Ещё не запускалось"}
          </span>
        </span>
        <Switch checked={p.enabled} onCheckedChange={(v) => setP({ ...p, enabled: v })} />
      </label>
      <div className="space-y-2">
        <div className="flex items-baseline justify-between text-sm">
          <span className="font-medium">Будет удалено</span>
          <span className={isFetching ? "text-muted-foreground" : ""}>
            {plan.remove_count} тегов · до {bytes(plan.reclaim)}
          </span>
        </div>
        <div className="max-h-72 space-y-3 overflow-auto rounded-lg border p-3">
          {repos.length === 0 && <div className="text-sm text-muted-foreground">Нечего удалять</div>}
          {repos.map((r) => (
            <div key={r.repo} className="text-xs">
              <div className="mb-1 flex items-center gap-2 font-mono font-medium">
                <Layers className="size-3.5 text-muted-foreground" />
                <span className="truncate">{r.repo}</span>
                <Badge variant="secondary">−{r.remove.length}</Badge>
              </div>
              <div className="space-y-0.5 pl-5 font-mono">
                {r.keep.map((k) => (
                  <div key={k.reference} className="flex justify-between gap-2 text-muted-foreground">
                    <span className="truncate">{k.reference.slice(r.repo.length) || k.reference}</span>
                    <span className="shrink-0 text-success">{REASON[k.reason]}</span>
                  </div>
                ))}
                {r.remove.map((k) => (
                  <div key={k.reference} className="flex justify-between gap-2 text-destructive/80 line-through decoration-destructive/40">
                    <span className="truncate">{k.reference.slice(r.repo.length) || k.reference.slice(0, 19)}</span>
                    <span className="shrink-0 no-underline">{ago(k.created)}</span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>
      </div>
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          Закрыть
        </Button>
        <Button variant="outline" disabled={!dirty} onClick={save}>
          Сохранить правило
        </Button>
        <Button variant="destructive" disabled={!plan.remove_count} onClick={run}>
          <Trash2 /> Удалить сейчас
        </Button>
      </DialogFooter>
    </>
  );
}

export function ImageRetentionButton() {
  const [open, setOpen] = useState(false);
  const { data } = useQuery({ queryKey: ["retention"], queryFn: () => api<RetentionResp>("/docker/images/retention"), enabled: open });
  return (
    <>
      <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
        <Layers /> Хранение образов
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent className="sm:max-w-2xl">
          <DialogHeader>
            <DialogTitle>Хранение образов</DialogTitle>
            <DialogDescription>
              В каждом репозитории остаются N самых новых образов. Образы контейнеров (включая остановленные) и защищённые теги не удаляются никогда.
            </DialogDescription>
          </DialogHeader>
          {data ? <Editor key={JSON.stringify(data.policy)} data={data} onClose={() => setOpen(false)} /> : <Skeleton className="h-80" />}
        </DialogContent>
      </Dialog>
    </>
  );
}
