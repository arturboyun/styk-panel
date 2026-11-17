"use client";

import { useRouter } from "next/navigation";
import { Box, Container as ContainerIcon, LogOut } from "lucide-react";
import { CommandDialog, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { NAV } from "@/components/nav";
import { Dot, containerTone, projectTone } from "@/components/status";
import { useProjects } from "@/lib/queries";

export function CommandPalette({ open, onOpenChange, onLogout }: { open: boolean; onOpenChange: (o: boolean) => void; onLogout: () => void }) {
  const router = useRouter();
  const { data } = useProjects(open);
  const go = (href: string) => {
    onOpenChange(false);
    router.push(href);
  };
  const containers = [...(data?.projects.flatMap((p) => p.containers) ?? []), ...(data?.standalone ?? [])];

  return (
    <CommandDialog open={open} onOpenChange={onOpenChange} title="Поиск">
      <CommandInput placeholder="Страница, проект, контейнер…" />
      <CommandList>
        <CommandEmpty>Ничего не найдено</CommandEmpty>
        <CommandGroup heading="Страницы">
          {NAV.flatMap((g) => g.items)
            .filter((i) => !i.soon)
            .map((i) => (
              <CommandItem key={i.href} value={`page ${i.label}`} onSelect={() => go(i.href)}>
                <i.icon /> {i.label}
              </CommandItem>
            ))}
        </CommandGroup>
        <CommandGroup heading="Проекты">
          {data?.projects.map((p) => (
            <CommandItem key={p.name} value={`project ${p.name}`} onSelect={() => go(`/projects/view/?name=${encodeURIComponent(p.name)}`)}>
              <Box /> {p.name}
              <Dot tone={projectTone(p)} className="ml-auto" />
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandGroup heading="Контейнеры · логи">
          {containers.map((c) => (
            <CommandItem key={c.id} value={`container ${c.name} ${c.image}`} onSelect={() => go(`/containers/view/?id=${c.id}&tab=logs`)}>
              <ContainerIcon /> {c.name}
              <span className="truncate text-xs text-muted-foreground">{c.image}</span>
              <Dot tone={containerTone(c)} className="ml-auto" />
            </CommandItem>
          ))}
        </CommandGroup>
        <CommandGroup heading="Аккаунт">
          <CommandItem
            value="logout выйти выход"
            onSelect={() => {
              onOpenChange(false);
              onLogout();
            }}
          >
            <LogOut /> Выйти
          </CommandItem>
        </CommandGroup>
      </CommandList>
    </CommandDialog>
  );
}
