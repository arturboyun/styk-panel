import { cn } from "@/lib/utils";
import type { Container, Project } from "@/lib/types";

type Tone = "ok" | "warn" | "bad" | "idle";

const TONE: Record<Tone, string> = {
  ok: "bg-success",
  warn: "bg-warning",
  bad: "bg-destructive",
  idle: "bg-muted-foreground/40",
};

export function Dot({ tone, pulse, className }: { tone: Tone; pulse?: boolean; className?: string }) {
  return (
    <span className={cn("relative inline-flex size-2 shrink-0", className)}>
      {pulse && <span className={cn("absolute inline-flex size-full animate-ping rounded-full opacity-60", TONE[tone])} />}
      <span className={cn("relative inline-flex size-2 rounded-full", TONE[tone])} />
    </span>
  );
}

export function containerTone(c: Pick<Container, "state" | "health">): Tone {
  if (c.state === "restarting") return "warn";
  if (c.state !== "running") return c.state === "exited" || c.state === "dead" ? "bad" : "idle";
  if (c.health === "unhealthy") return "bad";
  if (c.health === "starting") return "warn";
  return "ok";
}

export function projectTone(p: Project): Tone {
  if (p.status === "stopped") return "idle";
  if (p.containers.some((c) => containerTone(c) === "bad")) return "bad";
  if (p.status === "partial" || p.containers.some((c) => containerTone(c) === "warn")) return "warn";
  return "ok";
}

const STATE_RU: Record<string, string> = {
  running: "работает",
  exited: "остановлен",
  restarting: "перезапуск",
  paused: "пауза",
  created: "создан",
  dead: "мёртв",
};

export function ContainerState({ c }: { c: Pick<Container, "state" | "health"> }) {
  const tone = containerTone(c);
  return (
    <span className="inline-flex items-center gap-1.5 text-xs">
      <Dot tone={tone} />
      <span className={cn(tone === "bad" ? "text-destructive" : "text-muted-foreground")}>
        {c.health === "unhealthy" ? "unhealthy" : (STATE_RU[c.state] ?? c.state)}
        {c.health === "healthy" && " · healthy"}
      </span>
    </span>
  );
}

/** Thin usage bar that turns amber/red near the limit. */
export function Meter({ value, className }: { value: number; className?: string }) {
  const v = Math.max(0, Math.min(100, value));
  return (
    <div className={cn("h-1.5 w-full overflow-hidden rounded-full bg-muted", className)}>
      <div
        className={cn("h-full rounded-full transition-[width] duration-500", v > 90 ? "bg-destructive" : v > 75 ? "bg-warning" : "bg-primary")}
        style={{ width: `${v}%` }}
      />
    </div>
  );
}
