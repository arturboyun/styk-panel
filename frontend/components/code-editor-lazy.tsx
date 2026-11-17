"use client";

// CodeMirror is ~500 KB of JS; pages load it only when an editor is actually shown.
import dynamic from "next/dynamic";
import { Skeleton } from "@/components/ui/skeleton";

const loading = () => <Skeleton className="h-[min(60vh,32rem)] w-full rounded-lg" />;

export const CodeEditor = dynamic(() => import("@/components/code-editor").then((m) => m.CodeEditor), { ssr: false, loading });
export const DiffView = dynamic(() => import("@/components/code-editor").then((m) => m.DiffView), { ssr: false, loading });
