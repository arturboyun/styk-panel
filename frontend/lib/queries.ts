"use client";

import { useQuery } from "@tanstack/react-query";
import { api } from "@/lib/api";
import type { VersionInfo, AuditRow, ContainerInspect, HistoryPoint, Me, Overview, ProjectsResp, Range, SessionRow } from "@/lib/types";

export const useMe = () => useQuery({ queryKey: ["me"], queryFn: () => api<Me>("/auth/me"), staleTime: 60_000 });

/** `docker`: include Docker disk usage (slow; dashboard only). The sidebar needs just the hostname. */
export const useOverview = (docker = false) =>
  useQuery({
    queryKey: ["overview", docker],
    queryFn: () => api<Overview>(`/system/overview${docker ? "?docker=1" : ""}`),
    refetchInterval: docker ? 60_000 : false,
    staleTime: docker ? 30_000 : 5 * 60_000,
  });

export const useHistory = (range: Range) =>
  useQuery({
    queryKey: ["history", range],
    queryFn: () => api<HistoryPoint[]>(`/system/history?range=${range}`),
    refetchInterval: range === "1h" ? false : 60_000,
  });

export const useProjects = (enabled = true) =>
  useQuery({ queryKey: ["projects"], queryFn: () => api<ProjectsResp>("/docker/projects"), refetchInterval: 10_000, enabled });

export const useContainer = (id: string) =>
  useQuery({
    queryKey: ["container", id],
    queryFn: () => api<ContainerInspect>(`/docker/containers/${encodeURIComponent(id)}`),
    enabled: !!id,
    refetchInterval: 10_000,
  });

export const useAudit = (action?: string) =>
  useQuery({
    queryKey: ["audit", action],
    queryFn: () => api<AuditRow[]>(`/audit?limit=200${action ? `&action=${encodeURIComponent(action)}` : ""}`),
  });

export const useSessions = () => useQuery({ queryKey: ["sessions"], queryFn: () => api<SessionRow[]>("/auth/sessions") });

/** Versions and the newest release; the server caches the GitHub check for 6 h. */
export const useVersion = () => useQuery({ queryKey: ["version"], queryFn: () => api<VersionInfo>("/system/version"), staleTime: 30 * 60_000 });
