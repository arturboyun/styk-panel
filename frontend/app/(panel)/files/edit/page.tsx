"use client";

import Link from "next/link";
import { Suspense, useEffect, useState } from "react";
import { useSearchParams } from "next/navigation";
import { useQuery } from "@tanstack/react-query";
import { ChevronLeft, Download, FileWarning, History, Lock, RotateCcw, Save } from "lucide-react";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Skeleton } from "@/components/ui/skeleton";
import { CodeEditor, DiffView } from "@/components/code-editor-lazy";
import { changedLines } from "@/lib/diff";
import { Empty, PageHeader } from "@/components/page";
import { api, post } from "@/lib/api";
import { ago, bytes } from "@/lib/format";
import { Cancelled, errorMessage, useAction, useDownload, useOps } from "@/lib/ops";
import type { FileContent, FsEntry, FsVersion } from "@/lib/types";

function Editor() {
  const path = useSearchParams().get("path") ?? "";
  const { sudo } = useOps();
  const act = useAction();
  const download = useDownload();
  const file = useQuery({
    queryKey: ["file", path],
    queryFn: () => sudo(() => api<FileContent>(`/files/read?path=${encodeURIComponent(path)}`)),
    enabled: !!path,
    refetchOnWindowFocus: false,
  });
  const history = useQuery({ queryKey: ["file-history", path], queryFn: () => api<FsVersion[]>(`/files/history?path=${encodeURIComponent(path)}`), enabled: !!path });
  const [base, setBase] = useState<{ content: string; mtime: number } | null>(null);
  const [text, setText] = useState("");
  const [review, setReview] = useState(false);
  const [compare, setCompare] = useState<{ version: FsVersion; content: string } | null>(null);

  const [loaded, setLoaded] = useState<FileContent | null>(null);
  if (file.data && file.data !== loaded) {
    setLoaded(file.data);
    if (file.data.content != null) {
      setBase({ content: file.data.content, mtime: file.data.entry.mtime });
      setText(file.data.content);
    }
  }

  const name = path.split("/").pop() ?? path;
  const dirty = base != null && text !== base.content;
  const dir = path.slice(0, path.lastIndexOf("/")) || "/";

  useEffect(() => {
    const h = (e: BeforeUnloadEvent) => {
      if (dirty) e.preventDefault();
    };
    window.addEventListener("beforeunload", h);
    return () => window.removeEventListener("beforeunload", h);
  }, [dirty]);

  const save = async () => {
    const res = await act(() => post<FsEntry>("/files/write", { path, content: text, expected_mtime: base?.mtime }), { success: "Сохранено" });
    if (res) {
      setBase({ content: text, mtime: res.mtime });
      setReview(false);
      history.refetch();
    }
  };

  const openVersion = async (v: FsVersion) => {
    try {
      const r = await sudo(() => api<{ content: string }>(`/files/version?path=${encodeURIComponent(path)}&version=${encodeURIComponent(v.path)}`));
      setCompare({ version: v, content: r.content });
    } catch (e) {
      if (!(e instanceof Cancelled)) setCompare(null);
    }
  };

  const back = (
    <Link href={`/files/?path=${encodeURIComponent(dir)}`} className="mb-3 inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground">
      <ChevronLeft className="size-4" /> {dir}
    </Link>
  );

  if (file.isLoading) return <Skeleton className="h-[70vh]" />;
  if (file.error)
    return (
      <>
        {back}
        <Empty icon={<Lock />} title="Файл не открыт">
          {file.error instanceof Cancelled ? "Секретный файл: нужен код TOTP." : errorMessage(file.error)}
          <div className="mt-4">
            <Button variant="outline" onClick={() => file.refetch()}>
              Попробовать снова
            </Button>
          </div>
        </Empty>
      </>
    );
  const f = file.data!;
  if (f.too_large || f.binary)
    return (
      <>
        {back}
        <Empty icon={<FileWarning />} title={f.too_large ? "Файл слишком большой для редактора" : "Двоичный файл"}>
          {bytes(f.entry.size)}. Его можно скачать.
          <div className="mt-4">
            <Button onClick={() => download(path)}>
              <Download /> Скачать
            </Button>
          </div>
        </Empty>
      </>
    );

  const diff = changedLines(base?.content ?? "", text);
  const caddy = /caddyfile$/i.test(name);

  return (
    <>
      {back}
      <PageHeader
        title={
          <span className="flex items-center gap-2 font-mono text-xl">
            {name}
            {dirty && <Badge variant="secondary">изменён</Badge>}
            {!f.entry.writable && (
              <Badge variant="outline" className="gap-1 font-sans font-normal">
                <Lock className="size-3" /> только чтение
              </Badge>
            )}
          </span>
        }
        description={
          <span className="font-mono text-xs">
            {path} · {bytes(f.entry.size)} · {f.entry.mode} · изменён {ago(f.entry.mtime)}
          </span>
        }
        actions={
          <>
            <Button variant="outline" size="sm" onClick={() => download(path)}>
              <Download />
            </Button>
            {f.entry.writable && (
              <Button size="sm" disabled={!dirty} onClick={() => setReview(true)}>
                <Save /> Сохранить
              </Button>
            )}
          </>
        }
      />
      {caddy && (
        <Alert className="mb-4">
          <AlertTitle>Это общий Caddyfile</AlertTitle>
          <AlertDescription>
            Сохранение здесь не перезагружает Caddy. Для правок с проверкой <code>caddy validate</code>, reload и автооткатом используйте{" "}
            <Link href="/domains/" className="underline">
              Домены → Caddyfile
            </Link>
            .
          </AlertDescription>
        </Alert>
      )}
      <div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_18rem]">
        <CodeEditor value={text} onChange={setText} path={path} readOnly={!f.entry.writable} />
        <Card className="h-fit">
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-sm">
              <History className="size-4" /> История версий
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-1">
            {!history.data?.length && <div className="text-sm text-muted-foreground">Версий пока нет: они появляются при каждом сохранении из панели.</div>}
            {history.data?.map((v) => (
              <button key={v.path} onClick={() => openVersion(v)} className="flex w-full items-center justify-between rounded-md px-2 py-1.5 text-left text-sm hover:bg-muted">
                <span>{new Date(v.ts).toLocaleString("ru", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" })}</span>
                <span className="text-xs text-muted-foreground">{bytes(v.size)}</span>
              </button>
            ))}
          </CardContent>
        </Card>
      </div>

      <Dialog open={review} onOpenChange={setReview}>
        <DialogContent className="sm:max-w-5xl">
          <DialogHeader>
            <DialogTitle>Проверьте изменения</DialogTitle>
            <DialogDescription>
              <span className="text-success">+{diff.added}</span> <span className="text-destructive">−{diff.removed}</span> строк. Текущая версия сохранится в истории, файл
              перезапишется на месте (inode не меняется).
            </DialogDescription>
          </DialogHeader>
          {review && <DiffView before={base?.content ?? ""} after={text} path={path} />}
          <DialogFooter>
            <Button variant="outline" onClick={() => setReview(false)}>
              Назад
            </Button>
            <Button onClick={save}>
              <Save /> Записать
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={!!compare} onOpenChange={(o) => !o && setCompare(null)}>
        <DialogContent className="sm:max-w-5xl">
          <DialogHeader>
            <DialogTitle>Версия от {compare && new Date(compare.version.ts).toLocaleString("ru")}</DialogTitle>
            <DialogDescription>Слева сохранённая версия, справа текст в редакторе</DialogDescription>
          </DialogHeader>
          {compare && <DiffView before={compare.content} after={text} path={path} />}
          <DialogFooter>
            <Button variant="outline" onClick={() => setCompare(null)}>
              Закрыть
            </Button>
            {f.entry.writable && (
              <Button
                onClick={() => {
                  setText(compare!.content);
                  setCompare(null);
                }}
              >
                <RotateCcw /> Вернуть в редактор
              </Button>
            )}
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  );
}

export default function Page() {
  return (
    <Suspense>
      <Editor />
    </Suspense>
  );
}
