"use client";

import { useEffect, useRef } from "react";
import uPlot from "uplot";

export interface Series {
  label: string;
  /** CSS variable name, e.g. "--chart-1". */
  color: string;
  values: (number | null)[];
  fill?: boolean;
}

interface Props {
  /** Unix seconds. */
  x: number[];
  series: Series[];
  height?: number;
  yFormat?: (v: number) => string;
  yMax?: number;
}

function cssVar(name: string) {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

function withAlpha(color: string, alpha: number) {
  return color.startsWith("oklch(") ? color.replace(/\)$/, ` / ${alpha})`) : color;
}

export function TimeChart({ x, series, height = 180, yFormat = (v) => String(v), yMax }: Props) {
  const ref = useRef<HTMLDivElement>(null);
  const plot = useRef<uPlot | null>(null);
  const fmt = useRef(yFormat);
  useEffect(() => {
    fmt.current = yFormat;
  });
  const shape = series.map((s) => s.label + s.color).join("|") + yMax;

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const grid = cssVar("--border");
    const muted = cssVar("--muted-foreground");
    const opts: uPlot.Options = {
      width: el.clientWidth,
      height,
      padding: [8, 4, 0, 0],
      cursor: { points: { size: 6 }, drag: { x: false, y: false } },
      legend: { show: false },
      scales: { x: { time: true }, y: { range: (_u, _min, max) => [0, yMax ?? Math.max(max * 1.15, 1)] } },
      axes: [
        { stroke: muted, grid: { stroke: grid, width: 1 }, ticks: { show: false }, font: "11px sans-serif" },
        {
          stroke: muted,
          grid: { stroke: grid, width: 1 },
          ticks: { show: false },
          font: "11px sans-serif",
          size: 56,
          values: (_u, vals) => vals.map((v) => fmt.current(v)),
        },
      ],
      series: [
        {},
        ...series.map((s) => {
          const c = cssVar(s.color);
          return {
            label: s.label,
            stroke: c,
            width: 1.6,
            fill: s.fill === false ? undefined : withAlpha(c, 0.12),
            points: { show: false },
            value: (_u: uPlot, v: number | null) => (v == null ? "—" : fmt.current(v)),
          };
        }),
      ],
    };
    plot.current = new uPlot(opts, [x, ...series.map((s) => s.values)] as uPlot.AlignedData, el);
    const ro = new ResizeObserver(() => plot.current?.setSize({ width: el.clientWidth, height }));
    ro.observe(el);
    return () => {
      ro.disconnect();
      plot.current?.destroy();
      plot.current = null;
    };
    // Rebuild only when the series layout changes; data updates go through setData below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [shape, height]);

  useEffect(() => {
    plot.current?.setData([x, ...series.map((s) => s.values)] as uPlot.AlignedData);
  }, [x, series]);

  return <div ref={ref} className="w-full" />;
}

/** Tiny inline area chart without axes. */
export function Sparkline({ values, color = "--chart-1", height = 36, max }: { values: number[]; color?: string; height?: number; max?: number }) {
  if (values.length < 2) return <div style={{ height }} />;
  const top = max ?? Math.max(...values, 1) * 1.1;
  const w = 100;
  const pts = values.map((v, i) => `${(i / (values.length - 1)) * w},${height - (Math.min(v, top) / top) * height}`);
  return (
    <svg viewBox={`0 0 ${w} ${height}`} preserveAspectRatio="none" className="w-full" style={{ height }}>
      <polygon points={`0,${height} ${pts.join(" ")} ${w},${height}`} style={{ fill: `var(${color})`, opacity: 0.14 }} />
      <polyline points={pts.join(" ")} fill="none" style={{ stroke: `var(${color})` }} strokeWidth={1.5} vectorEffect="non-scaling-stroke" />
    </svg>
  );
}
