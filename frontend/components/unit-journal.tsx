"use client";

import { useQuery } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";

export function UnitJournalSheet({ unit, onClose }: { unit: string | null; onClose: () => void }) {
  const { data, isFetching, refetch, error } = useQuery({
    queryKey: ["journal", unit],
    queryFn: () => api<string[]>(`/system/units/${encodeURIComponent(unit!)}/journal?lines=400`),
    enabled: !!unit,
  });
  return (
    <Sheet open={!!unit} onOpenChange={(o) => !o && onClose()}>
      <SheetContent side="right" className="w-full gap-0 sm:max-w-3xl">
        <SheetHeader className="flex-row items-center justify-between space-y-0 pr-12">
          <div>
            <SheetTitle className="font-mono">{unit}</SheetTitle>
            <SheetDescription>journalctl, последние 400 строк</SheetDescription>
          </div>
          <Button variant="outline" size="sm" onClick={() => refetch()} disabled={isFetching}>
            <RefreshCw className={cn(isFetching && "animate-spin")} />
          </Button>
        </SheetHeader>
        <div className="min-h-0 flex-1 overflow-auto bg-zinc-950 p-4 font-mono text-xs leading-relaxed text-zinc-200">
          {error && <div className="text-red-400">{error.message}</div>}
          {data?.length === 0 && <div className="text-zinc-500">Записей нет</div>}
          {data?.map((l, i) => (
            <div key={i} className={cn("whitespace-pre-wrap break-all", /FAIL|error|Failed/i.test(l) && "text-red-400")}>
              {l}
            </div>
          ))}
        </div>
      </SheetContent>
    </Sheet>
  );
}
