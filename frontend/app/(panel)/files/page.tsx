"use client";

import Link from "next/link";
import { Suspense, useMemo, useRef, useState } from "react";
import { useRouter, useSearchParams } from "next/navigation";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Download, File, FileCode, Folder, FolderPlus, Link2, Lock, MoreHorizontal, Pencil, Search, Trash2, Upload } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { useConfirm } from "@/components/confirm";
import { Empty, PageHeader } from "@/components/page";
import { ApiError, api, post } from "@/lib/api";
import { ago, bytes } from "@/lib/format";
import { Cancelled, errorMessage, useAction, useDownload, useOps } from "@/lib/ops";
import type { FsEntry, FsListing } from "@/lib/types";
import { cn } from "@/lib/utils";

const CHUNK = 1 << 20;

function parent(p: string) {
  const i = p.lastIndexOf("/");
  return i <= 0 ? "/" : p.slice(0, i);
}

function NameDialog({ title, initial, onSubmit, onClose }: { title: string | null; initial: string; onSubmit: (v: string) => void; onClose: () => void }) {
  const [v, setV] = useState(initial);
  return (
    <Dialog open={!!title} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <Input autoFocus value={v} onChange={(e) => setV(e.target.value)} onKeyDown={(e) => e.key === "Enter" && v.trim() && onSubmit(v.trim())} />
        <DialogFooter>
          <Button variant="outline" onClick={onClose}>
            Отмена
          </Button>
          <Button disabled={!v.trim() || v.includes("/")} onClick={() => onSubmit(v.trim())}>
            Готово
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function Browser() {
  const params = useSearchParams();
  const router = useRouter();
  const qc = useQueryClient();
  const roots = useQuery({ queryKey: ["fs-roots"], queryFn: () => api<string[]>("/files/roots"), staleTime: Infinity });
  const path = params.get("path") ?? roots.data?.[0] ?? "";
  const { data, error, isLoading } = useQuery({
    queryKey: ["fs", path],
    queryFn: () => api<FsListing>(`/files/list?path=${encodeURIComponent(path)}`),
    enabled: !!path,
  });
  const [filter, setFilter] = useState("");
  const [dialog, setDialog] = useState<{ kind: "mkdir" | "rename"; entry?: FsEntry } | null>(null);
  const [upload, setUpload] = useState<{ name: string; pct: number } | null>(null);
  const [drag, setDrag] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const act = useAction();
  const confirm = useConfirm();
  const download = useDownload();
  const { sudo } = useOps();

  const go = (p: string) => router.push(`/files/?path=${encodeURIComponent(p)}`);
  const refresh = () => qc.invalidateQueries({ queryKey: ["fs", path] });
  const root = data?.root ?? roots.data?.find((r) => path.startsWith(r)) ?? "";
  const crumbs = useMemo(() => {
    if (!root) return [];
    const rest = path.slice(root.length).split("/").filter(Boolean);
    return [{ name: root, path: root }, ...rest.map((n, i) => ({ name: n, path: `${root}/${rest.slice(0, i + 1).join("/")}` }))];
  }, [path, root]);
  const entries = (data?.entries ?? []).filter((e) => !filter || e.name.toLowerCase().includes(filter.toLowerCase()));

  const uploadFiles = async (files: FileList | File[]) => {
    for (const f of Array.from(files)) {
      const target = `${data!.path}/${f.name}`;
      try {
        if (data!.entries.some((e) => e.name === f.name)) {
          const ok = await confirm({ title: `Заменить ${f.name}?`, description: "Прежняя версия сохранится в истории (для файлов до 20 МБ).", confirmText: "Заменить" });
          if (!ok) continue;
        }
        setUpload({ name: f.name, pct: 0 });
        for (let off = 0; off < Math.max(f.size, 1); off += CHUNK) {
          const body = f.slice(off, off + CHUNK);
          const last = off + CHUNK >= f.size;
          await sudo(() =>
            api(`/files/upload?path=${encodeURIComponent(target)}&offset=${off}&last=${last}`, { method: "POST", body, headers: { "content-type": "application/octet-stream" } }),
          );
          setUpload({ name: f.name, pct: Math.min(100, ((off + CHUNK) / Math.max(f.size, 1)) * 100) });
          if (f.size === 0) break;
        }
        toast.success(`${f.name} загружен`);
      } catch (e) {
        if (!(e instanceof Cancelled)) toast.error(`${f.name}: ${errorMessage(e)}`);
        break;
      }
    }
    setUpload(null);
    refresh();
  };

  const trash = async (e: FsEntry) => {
    const ok = await confirm({ title: `Удалить ${e.name}?`, description: "Файл переместится в корзину панели (/opt/styk-panel/data/trash) и хранится 30 дней.", destructive: true, confirmText: "В корзину" });
    if (ok) await act(() => post("/files/trash", { path: e.path }), { success: "Перемещено в корзину" }).then(refresh);
  };

  if (roots.isLoading) return <Skeleton className="h-96" />;

  return (
    <>
      <PageHeader
        title="Файлы"
        description="Песочница агента: всё, что вне этих каталогов, недоступно. Изменения требуют sudo, прошлые версии сохраняются."
        actions={
          data?.writable && (
            <>
              <Button variant="outline" size="sm" onClick={() => setDialog({ kind: "mkdir" })}>
                <FolderPlus /> Папка
              </Button>
              <Button size="sm" onClick={() => fileInput.current?.click()}>
                <Upload /> Загрузить
              </Button>
              <input ref={fileInput} type="file" multiple hidden onChange={(e) => e.target.files && uploadFiles(e.target.files)} />
            </>
          )
        }
      />
      <Tabs value={root} onValueChange={(v) => go(String(v))} className="mb-4">
        <TabsList>
          {roots.data?.map((r) => (
            <TabsTrigger key={r} value={r} className="font-mono text-xs">
              {r.split("/").slice(-2).join("/").replace(/^\.dev\/fsroot\//, "")}
            </TabsTrigger>
          ))}
        </TabsList>
      </Tabs>
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <nav className="flex min-w-0 flex-1 flex-wrap items-center gap-0.5 font-mono text-sm">
          {crumbs.map((c, i) => (
            <span key={c.path} className="flex items-center">
              {i > 0 && <ChevronRight className="size-3.5 text-muted-foreground" />}
              <button onClick={() => go(c.path)} className={cn("rounded px-1 hover:bg-muted", i === crumbs.length - 1 && "font-semibold")}>
                {c.name}
              </button>
            </span>
          ))}
          {data && !data.writable && (
            <Badge variant="outline" className="ml-2 gap-1 font-sans font-normal">
              <Lock className="size-3" /> только чтение
            </Badge>
          )}
        </nav>
        <div className="relative w-56">
          <Search className="absolute top-2.5 left-2.5 size-4 text-muted-foreground" />
          <Input value={filter} onChange={(e) => setFilter(e.target.value)} placeholder="Фильтр" className="pl-8" />
        </div>
      </div>
      {upload && (
        <div className="mb-3 flex items-center gap-3 text-sm">
          <span className="truncate">{upload.name}</span>
          <Progress value={upload.pct} className="w-64" />
        </div>
      )}
      {error ? (
        <Empty title="Нет доступа">{error instanceof ApiError ? error.message : String(error)}</Empty>
      ) : isLoading || !data ? (
        <Skeleton className="h-96" />
      ) : (
        <Card
          className={cn("py-0 transition", drag && "ring-2 ring-primary")}
          onDragOver={(e) => {
            if (!data.writable) return;
            e.preventDefault();
            setDrag(true);
          }}
          onDragLeave={() => setDrag(false)}
          onDrop={(e) => {
            e.preventDefault();
            setDrag(false);
            if (data.writable && e.dataTransfer.files.length) uploadFiles(e.dataTransfer.files);
          }}
        >
          <CardContent className="px-0">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="pl-4">Имя</TableHead>
                  <TableHead className="text-right">Размер</TableHead>
                  <TableHead className="hidden sm:table-cell">Изменён</TableHead>
                  <TableHead className="hidden font-mono md:table-cell">Права</TableHead>
                  <TableHead className="w-10 pr-4" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.path !== data.root && (
                  <TableRow className="cursor-pointer" onClick={() => go(parent(data.path))}>
                    <TableCell className="pl-4 text-muted-foreground" colSpan={5}>
                      ..
                    </TableCell>
                  </TableRow>
                )}
                {entries.map((e) => {
                  const Icon = e.kind === "dir" ? Folder : e.kind === "symlink" ? Link2 : /\.(ya?ml|json|toml|env|conf|sh|caddy)$|Caddyfile|Dockerfile/i.test(e.name) ? FileCode : File;
                  const open = () => (e.kind === "dir" ? go(e.path) : router.push(`/files/edit/?path=${encodeURIComponent(e.path)}`));
                  return (
                    <TableRow key={e.path} className="cursor-pointer" onDoubleClick={open}>
                      <TableCell className="pl-4">
                        <button onClick={open} className="flex items-center gap-2 text-left hover:text-primary">
                          <Icon className={cn("size-4 shrink-0", e.kind === "dir" ? "text-primary" : "text-muted-foreground")} />
                          <span className="truncate">{e.name}</span>
                        </button>
                      </TableCell>
                      <TableCell className="text-right text-xs tabular text-muted-foreground">{e.kind === "dir" ? "—" : bytes(e.size)}</TableCell>
                      <TableCell className="hidden text-xs text-muted-foreground sm:table-cell">{ago(e.mtime)}</TableCell>
                      <TableCell className="hidden font-mono text-xs text-muted-foreground md:table-cell">
                        {e.mode} {e.uid === 0 ? "root" : e.uid}
                      </TableCell>
                      <TableCell className="pr-4">
                        <DropdownMenu>
                          <DropdownMenuTrigger render={<Button variant="ghost" size="icon-sm" aria-label="Действия" />}>
                            <MoreHorizontal />
                          </DropdownMenuTrigger>
                          <DropdownMenuContent align="end">
                            {e.kind !== "dir" && (
                              <>
                                <DropdownMenuItem render={<Link href={`/files/edit/?path=${encodeURIComponent(e.path)}`} />}>
                                  <Pencil /> Открыть
                                </DropdownMenuItem>
                                <DropdownMenuItem onClick={() => download(e.path)}>
                                  <Download /> Скачать
                                </DropdownMenuItem>
                              </>
                            )}
                            {data.writable && (
                              <>
                                <DropdownMenuItem onClick={() => setDialog({ kind: "rename", entry: e })}>
                                  <Pencil /> Переименовать
                                </DropdownMenuItem>
                                <DropdownMenuSeparator />
                                <DropdownMenuItem variant="destructive" onClick={() => trash(e)}>
                                  <Trash2 /> В корзину
                                </DropdownMenuItem>
                              </>
                            )}
                          </DropdownMenuContent>
                        </DropdownMenu>
                      </TableCell>
                    </TableRow>
                  );
                })}
              </TableBody>
            </Table>
            {entries.length === 0 && <div className="py-10 text-center text-sm text-muted-foreground">Пусто</div>}
            {data.truncated && <div className="py-2 text-center text-xs text-muted-foreground">Показаны первые 5000 записей</div>}
          </CardContent>
        </Card>
      )}
      <NameDialog
        key={dialog ? `${dialog.kind}${dialog.entry?.path}` : "none"}
        title={dialog ? (dialog.kind === "mkdir" ? "Новая папка" : `Переименовать ${dialog.entry?.name}`) : null}
        initial={dialog?.entry?.name ?? ""}
        onClose={() => setDialog(null)}
        onSubmit={async (name) => {
          const d = dialog!;
          setDialog(null);
          if (d.kind === "mkdir") await act(() => post("/files/mkdir", { path: `${data!.path}/${name}` }), { success: "Папка создана" });
          else await act(() => post("/files/rename", { from: d.entry!.path, to: `${parent(d.entry!.path)}/${name}` }), { success: "Переименовано" });
          refresh();
        }}
      />
    </>
  );
}

export default function Page() {
  return (
    <Suspense>
      <Browser />
    </Suspense>
  );
}
