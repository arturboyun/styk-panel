import { cn } from "@/lib/utils";

// The «стык» mark: two brackets that overlap. Drawn in currentColor so it sits on the brand tile.
export function LogoMark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2.6} strokeLinecap="round" strokeLinejoin="round" aria-hidden className={cn("size-4", className)}>
      <path d="M4.5 15.5V4.5h11" />
      <path d="M19.5 8.5v11h-11" />
    </svg>
  );
}
