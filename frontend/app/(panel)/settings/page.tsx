"use client";

import { KeyRound, Laptop, Send, Smartphone } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { PageHeader } from "@/components/page";
import { UpdatesCard } from "@/components/updates-card";
import { ago, dateTime } from "@/lib/format";
import { useMe, useSessions } from "@/lib/queries";

function device(ua: string | null) {
  if (!ua) return "Неизвестно";
  const browser = /Firefox\//.test(ua) ? "Firefox" : /Edg\//.test(ua) ? "Edge" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "Браузер";
  const os = /Android/.test(ua) ? "Android" : /iPhone|iPad/.test(ua) ? "iOS" : /Mac OS/.test(ua) ? "macOS" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "";
  return `${browser}${os ? ` · ${os}` : ""}`;
}

export default function SettingsPage() {
  const { data: me } = useMe();
  const { data: sessions, isLoading } = useSessions();

  return (
    <>
      <PageHeader title="Настройки" description="Учётная запись, сессии и обновления" />
      <div className="grid gap-4 lg:grid-cols-3">
        <UpdatesCard />
        <Card>
          <CardHeader>
            <CardTitle>Учётная запись</CardTitle>
            <CardDescription>Пользователями управляет CLI на сервере</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <div className="flex justify-between">
              <span className="text-muted-foreground">Логин</span>
              <span className="font-medium">{me?.username}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">2FA</span>
              <Badge variant="outline" className="gap-1">
                <KeyRound className="size-3" /> TOTP включена
              </Badge>
            </div>
            <div className="flex justify-between">
              <span className="text-muted-foreground">Текущий вход</span>
              <span>{me?.method === "telegram" ? "Telegram" : "пароль + TOTP"}</span>
            </div>
            <div className="rounded-lg bg-muted/60 p-3 text-xs text-muted-foreground">
              <div className="mb-1 flex items-center gap-1.5 font-medium text-foreground">
                <Send className="size-3.5" /> Вход через Telegram
              </div>
              Привязать аккаунт: <code className="font-mono">styk-panel user link-telegram {me?.username} &lt;telegram_id&gt;</code>
            </div>
          </CardContent>
        </Card>
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Активные сессии</CardTitle>
            <CardDescription>Сессия живёт 7 дней и завершается после 12 часов бездействия</CardDescription>
          </CardHeader>
          <CardContent className="space-y-2">
            {isLoading && <Skeleton className="h-24" />}
            {sessions?.map((s, i) => (
              <div key={i} className="flex items-center gap-3 rounded-lg border p-3">
                <div className="grid size-9 place-items-center rounded-lg bg-muted">
                  {/Android|iPhone|iPad/.test(s.user_agent ?? "") ? <Smartphone className="size-4" /> : <Laptop className="size-4" />}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 text-sm font-medium">
                    {device(s.user_agent)}
                    {s.current && <Badge>эта сессия</Badge>}
                  </div>
                  <div className="text-xs text-muted-foreground">
                    {s.ip} · {s.method === "telegram" ? "Telegram" : "пароль + TOTP"} · вход {dateTime(s.created_at)}
                  </div>
                </div>
                <div className="text-xs text-muted-foreground">{ago(s.last_seen)}</div>
              </div>
            ))}
          </CardContent>
        </Card>
      </div>
    </>
  );
}
