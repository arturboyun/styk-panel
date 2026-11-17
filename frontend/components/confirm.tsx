"use client";

import { createContext, useCallback, useContext, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";

export interface ConfirmOptions {
  title: string;
  description?: React.ReactNode;
  confirmText?: string;
  destructive?: boolean;
  /** The user must type this exact text to enable the button. */
  typeToConfirm?: string;
}

const Ctx = createContext<((o: ConfirmOptions) => Promise<boolean>) | null>(null);

export function ConfirmProvider({ children }: { children: React.ReactNode }) {
  const [opts, setOpts] = useState<ConfirmOptions | null>(null);
  const [typed, setTyped] = useState("");
  const resolver = useRef<((v: boolean) => void) | null>(null);

  const confirm = useCallback(
    (o: ConfirmOptions) =>
      new Promise<boolean>((resolve) => {
        resolver.current = resolve;
        setTyped("");
        setOpts(o);
      }),
    [],
  );

  const close = (v: boolean) => {
    resolver.current?.(v);
    resolver.current = null;
    setOpts(null);
  };
  const blocked = !!opts?.typeToConfirm && typed !== opts.typeToConfirm;

  return (
    <Ctx.Provider value={confirm}>
      {children}
      <Dialog open={!!opts} onOpenChange={(o) => !o && close(false)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{opts?.title}</DialogTitle>
            {opts?.description && <DialogDescription render={<div />}>{opts.description}</DialogDescription>}
          </DialogHeader>
          {opts?.typeToConfirm && (
            <div className="space-y-2 text-sm">
              <div>
                Введите <code className="rounded bg-muted px-1 font-mono">{opts.typeToConfirm}</code> для подтверждения
              </div>
              <Input value={typed} autoFocus onChange={(e) => setTyped(e.target.value)} onKeyDown={(e) => e.key === "Enter" && !blocked && close(true)} />
            </div>
          )}
          <DialogFooter>
            <Button variant="outline" onClick={() => close(false)}>
              Отмена
            </Button>
            <Button variant={opts?.destructive ? "destructive" : "default"} disabled={blocked} onClick={() => close(true)}>
              {opts?.confirmText ?? "Подтвердить"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </Ctx.Provider>
  );
}

export function useConfirm() {
  const v = useContext(Ctx);
  if (!v) throw new Error("ConfirmProvider missing");
  return v;
}
