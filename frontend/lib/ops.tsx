"use client";

import { createContext, useCallback, useContext, useRef, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { ShieldCheck } from "lucide-react";
import { toast } from "sonner";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { InputOTP, InputOTPGroup, InputOTPSlot } from "@/components/ui/input-otp";
import { ConfirmProvider } from "@/components/confirm";
import { JobDialog } from "@/components/job-log";
import { ApiError, api, post } from "@/lib/api";
import type { Me } from "@/lib/types";

interface Ops {
  /** Runs `fn`; when the server asks for sudo, asks for a TOTP code and retries once. */
  sudo: <T>(fn: () => Promise<T>) => Promise<T>;
  openJob: (id: number, title: string) => void;
  /** Makes sure sudo mode is active (asks for a code if not); for plain links/downloads. */
  ensureSudo: () => Promise<void>;
}

const Ctx = createContext<Ops | null>(null);

export class Cancelled extends Error {}

export function OpsProvider({ children }: { children: React.ReactNode }) {
  const qc = useQueryClient();
  const [asking, setAsking] = useState(false);
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const waiter = useRef<{ resolve: () => void; reject: (e: Error) => void } | null>(null);
  const [job, setJob] = useState<{ id: number; title: string } | null>(null);

  const askSudo = useCallback(
    () =>
      new Promise<void>((resolve, reject) => {
        waiter.current = { resolve, reject };
        setCode("");
        setError(null);
        setAsking(true);
      }),
    [],
  );

  const submit = async (value: string) => {
    setBusy(true);
    setError(null);
    try {
      await post("/auth/sudo", { code: value });
      qc.invalidateQueries({ queryKey: ["me"] });
      setAsking(false);
      waiter.current?.resolve();
      waiter.current = null;
    } catch (e) {
      setError(e instanceof ApiError ? (e.code === "invalid_code" ? "Неверный код" : e.message) : "Ошибка");
      setCode("");
    } finally {
      setBusy(false);
    }
  };

  const sudo = useCallback(
    async <T,>(fn: () => Promise<T>): Promise<T> => {
      try {
        return await fn();
      } catch (e) {
        if (e instanceof ApiError && e.code === "sudo_required") {
          await askSudo();
          return fn();
        }
        throw e;
      }
    },
    [askSudo],
  );

  const openJob = useCallback((id: number, title: string) => setJob({ id, title }), []);

  const ensureSudo = useCallback(async () => {
    const me = await api<Me>("/auth/me");
    if (me.sudo_until && new Date(me.sudo_until).getTime() > Date.now() + 10_000) return;
    await askSudo();
  }, [askSudo]);

  return (
    <Ctx.Provider value={{ sudo, openJob, ensureSudo }}>
      <ConfirmProvider>{children}</ConfirmProvider>
      <Dialog
        open={asking}
        onOpenChange={(o) => {
          if (!o) {
            setAsking(false);
            waiter.current?.reject(new Cancelled("cancelled"));
            waiter.current = null;
          }
        }}
      >
        <DialogContent className="sm:max-w-sm">
          <DialogHeader>
            <DialogTitle className="flex items-center gap-2">
              <ShieldCheck className="size-5 text-primary" /> Подтвердите действие
            </DialogTitle>
            <DialogDescription>Введите код из приложения-аутентификатора. Потом 30 минут код не спросят, и каждое действие продлевает это время.</DialogDescription>
          </DialogHeader>
          <div className="flex flex-col items-center gap-3 py-2">
            <InputOTP
              maxLength={6}
              value={code}
              autoFocus
              disabled={busy}
              inputMode="numeric"
              pattern="^[0-9]*$"
              onChange={(v) => {
                setCode(v);
                if (v.length === 6) submit(v);
              }}
            >
              <InputOTPGroup>
                {Array.from({ length: 6 }, (_, i) => (
                  <InputOTPSlot key={i} index={i} className="size-11 text-lg" />
                ))}
              </InputOTPGroup>
            </InputOTP>
            <div className="h-5 text-sm text-destructive">{error}</div>
          </div>
        </DialogContent>
      </Dialog>
      <JobDialog job={job} onClose={() => setJob(null)} />
    </Ctx.Provider>
  );
}

export function useOps(): Ops {
  const v = useContext(Ctx);
  if (!v) throw new Error("OpsProvider missing");
  return v;
}

export function errorMessage(e: unknown): string {
  if (e instanceof ApiError) return e.message;
  if (e instanceof Error) return e.message;
  return "Ошибка";
}

/**
 * Wraps a mutating call: sudo on demand, toast on error, job dialog when the server
 * started a job. Returns the result, or undefined when cancelled/failed.
 */
export function useAction() {
  const { sudo, openJob } = useOps();
  const qc = useQueryClient();
  return useCallback(
    async <T,>(fn: () => Promise<T>, opts: { success?: string; job?: string; invalidate?: string[][] } = {}): Promise<T | undefined> => {
      try {
        const res = await sudo(fn);
        const id = (res as { job?: number } | undefined)?.job;
        if (typeof id === "number") openJob(id, opts.job ?? "Задача");
        else if (opts.success) toast.success(opts.success);
        opts.invalidate?.forEach((k) => qc.invalidateQueries({ queryKey: k }));
        return res;
      } catch (e) {
        if (!(e instanceof Cancelled)) toast.error(errorMessage(e));
        return undefined;
      }
    },
    [sudo, openJob, qc],
  );
}

/** Downloads a file from the sandboxed file API (asks for sudo when needed). */
export function useDownload() {
  const { ensureSudo } = useOps();
  return useCallback(
    async (path: string) => {
      try {
        await ensureSudo();
      } catch (e) {
        if (!(e instanceof Cancelled)) toast.error(errorMessage(e));
        return;
      }
      const a = document.createElement("a");
      a.href = `/api/files/download?path=${encodeURIComponent(path)}`;
      a.rel = "noopener";
      document.body.appendChild(a);
      a.click();
      a.remove();
    },
    [ensureSudo],
  );
}
