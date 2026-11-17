"use client";

import { useEffect } from "react";
import { useRouter } from "next/navigation";
import { Loader2 } from "lucide-react";
import { AppShell } from "@/components/app-shell";
import { ApiError } from "@/lib/api";
import { LiveProvider } from "@/lib/live";
import { OpsProvider } from "@/lib/ops";
import { useMe } from "@/lib/queries";

export default function PanelLayout({ children }: { children: React.ReactNode }) {
  const router = useRouter();
  const { data, error } = useMe();

  useEffect(() => {
    if (error instanceof ApiError && error.status === 401) router.replace("/login/");
  }, [error, router]);

  if (!data) {
    return (
      <div className="grid min-h-screen place-items-center text-muted-foreground">
        {error && !(error instanceof ApiError && error.status === 401) ? "Сервер недоступен" : <Loader2 className="animate-spin" />}
      </div>
    );
  }
  return (
    <LiveProvider>
      <OpsProvider>
        <AppShell>{children}</AppShell>
      </OpsProvider>
    </LiveProvider>
  );
}
