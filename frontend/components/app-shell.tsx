"use client";

import Link from "next/link";
import { usePathname, useRouter } from "next/navigation";
import { useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTheme } from "next-themes";
import { LogOut, Menu, Monitor, Moon, Search, Sun, UserRound } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Sheet, SheetContent, SheetTitle } from "@/components/ui/sheet";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { CommandPalette } from "@/components/command-palette";
import { LogoMark } from "@/components/logo";
import { NAV } from "@/components/nav";
import { Dot } from "@/components/status";
import { post } from "@/lib/api";
import { useLive } from "@/lib/live";
import { useMe, useOverview, useVersion } from "@/lib/queries";
import { cn } from "@/lib/utils";

function isActive(pathname: string, href: string) {
  return href === "/" ? pathname === "/" : pathname.startsWith(href.replace(/\/$/, ""));
}

function Sidebar({ onNavigate, onLogout }: { onNavigate?: () => void; onLogout: () => void }) {
  const pathname = usePathname();
  const { data } = useOverview();
  const { data: me } = useMe();
  const { data: version } = useVersion();
  return (
    <div className="flex h-full flex-col">
      <Link href="/" onClick={onNavigate} className="flex items-center gap-2.5 px-5 pt-5 pb-6">
        <div className="grid size-8 place-items-center rounded-lg bg-gradient-to-br from-primary to-chart-5 text-primary-foreground shadow-sm">
          <LogoMark className="size-[18px]" />
        </div>
        <div className="leading-tight">
          <div className="text-sm font-semibold">Styk Panel</div>
          <div className="text-xs text-muted-foreground">{data?.info?.hostname ?? "…"}</div>
          {version?.update_available && <div className="text-[11px] font-medium text-primary">есть {version.latest?.version}</div>}
        </div>
      </Link>
      <nav className="flex-1 space-y-5 overflow-y-auto px-3 pb-4">
        {NAV.map((group, i) => (
          <div key={i}>
            {group.title && <div className="px-2 pb-1.5 text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{group.title}</div>}
            <div className="space-y-0.5">
              {group.items.map((item) => {
                const active = isActive(pathname, item.href);
                const Icon = item.icon;
                if (item.soon) {
                  return (
                    <div key={item.href} className="flex cursor-not-allowed items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm text-muted-foreground/60">
                      <Icon className="size-4" />
                      {item.label}
                      <span className="ml-auto rounded bg-muted px-1.5 text-[10px]">скоро</span>
                    </div>
                  );
                }
                return (
                  <Link
                    key={item.href}
                    href={item.href}
                    onClick={onNavigate}
                    className={cn(
                      "flex items-center gap-2.5 rounded-lg px-2 py-1.5 text-sm transition-colors",
                      active ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground" : "text-sidebar-foreground/80 hover:bg-sidebar-accent/60",
                    )}
                  >
                    <Icon className={cn("size-4", active && "text-primary")} />
                    {item.label}
                  </Link>
                );
              })}
            </div>
          </div>
        ))}
      </nav>
      <div className="flex items-center gap-2 border-t px-4 py-3">
        <UserRound className="size-4 shrink-0 text-muted-foreground" />
        <Link href="/settings/" onClick={onNavigate} className="min-w-0 flex-1 truncate text-sm hover:underline">
          {me?.username ?? "…"}
        </Link>
        <Button variant="ghost" size="sm" onClick={onLogout} className="text-muted-foreground hover:text-destructive">
          <LogOut /> Выйти
        </Button>
      </div>
    </div>
  );
}

function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  const next = theme === "dark" ? "light" : theme === "light" ? "system" : "dark";
  const Icon = theme === "dark" ? Moon : theme === "light" ? Sun : Monitor;
  return (
    <Tooltip>
      <TooltipTrigger render={<Button variant="ghost" size="icon" onClick={() => setTheme(next)} aria-label="Тема" />}>
        <Icon />
      </TooltipTrigger>
      <TooltipContent>Тема: {theme === "dark" ? "тёмная" : theme === "light" ? "светлая" : "системная"}</TooltipContent>
    </Tooltip>
  );
}

export function AppShell({ children }: { children: React.ReactNode }) {
  const [mobileOpen, setMobileOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const { connected } = useLive();
  const { data: me } = useMe();
  const router = useRouter();
  const qc = useQueryClient();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((o) => !o);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const logout = async () => {
    await post("/auth/logout").catch(() => {});
    qc.clear();
    router.replace("/login/");
  };

  return (
    <div className="flex min-h-screen">
      <aside className="sticky top-0 hidden h-screen w-60 shrink-0 border-r bg-sidebar lg:block">
        <Sidebar onLogout={logout} />
      </aside>
      <Sheet open={mobileOpen} onOpenChange={setMobileOpen}>
        <SheetContent side="left" className="w-64 bg-sidebar p-0">
          <SheetTitle className="sr-only">Навигация</SheetTitle>
          <Sidebar onNavigate={() => setMobileOpen(false)} onLogout={logout} />
        </SheetContent>
      </Sheet>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="sticky top-0 z-30 flex h-14 items-center gap-2 border-b bg-background/80 px-4 backdrop-blur lg:px-8">
          <Button variant="ghost" size="icon" className="lg:hidden" onClick={() => setMobileOpen(true)} aria-label="Меню">
            <Menu />
          </Button>
          <button
            onClick={() => setPaletteOpen(true)}
            className="flex h-8 min-w-0 flex-1 max-w-sm items-center gap-2 rounded-lg border bg-muted/40 px-3 text-sm text-muted-foreground transition-colors hover:bg-muted"
          >
            <Search className="size-4" />
            <span className="flex-1 truncate text-left">Поиск и команды…</span>
            <kbd className="hidden rounded border bg-background px-1.5 font-mono text-[10px] sm:inline">Ctrl K</kbd>
          </button>
          <div className="ml-auto flex shrink-0 items-center gap-1">
            <Tooltip>
              <TooltipTrigger render={<div className="mr-2 flex items-center gap-2 text-xs text-muted-foreground" />}>
                <Dot tone={connected ? "ok" : "warn"} pulse={connected} />
                <span className="hidden sm:inline">{connected ? "live" : "переподключение"}</span>
              </TooltipTrigger>
              <TooltipContent>{connected ? "Метрики обновляются каждые 5 секунд" : "Нет соединения с сервером"}</TooltipContent>
            </Tooltip>
            <ThemeToggle />
            <DropdownMenu>
              <DropdownMenuTrigger render={<Button variant="ghost" size="sm" className="gap-2" />}>
                <UserRound />
                <span className="hidden sm:inline">{me?.username}</span>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-52">
                <DropdownMenuLabel>
                  {me?.username}
                  <div className="text-xs font-normal text-muted-foreground">вход через {me?.method === "telegram" ? "Telegram" : "пароль + TOTP"}</div>
                </DropdownMenuLabel>
                <DropdownMenuSeparator />
                <DropdownMenuItem onClick={() => router.push("/settings/")}>Сессии и настройки</DropdownMenuItem>
                <DropdownMenuItem variant="destructive" onClick={logout}>
                  <LogOut /> Выйти
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </header>
        <main className="mx-auto w-full max-w-[1400px] flex-1 px-4 py-6 lg:px-8 lg:py-8">{children}</main>
      </div>
      <CommandPalette open={paletteOpen} onOpenChange={setPaletteOpen} onLogout={logout} />
    </div>
  );
}
