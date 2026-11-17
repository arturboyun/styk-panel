"use client";

import { useEffect, useMemo, useRef } from "react";
import CodeMirror from "@uiw/react-codemirror";
import { MergeView } from "@codemirror/merge";
import { json } from "@codemirror/lang-json";
import { yaml } from "@codemirror/lang-yaml";
import { StreamLanguage } from "@codemirror/language";
import { dockerFile } from "@codemirror/legacy-modes/mode/dockerfile";
import { javascript } from "@codemirror/legacy-modes/mode/javascript";
import { nginx } from "@codemirror/legacy-modes/mode/nginx";
import { properties } from "@codemirror/legacy-modes/mode/properties";
import { python } from "@codemirror/legacy-modes/mode/python";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { oneDark } from "@codemirror/theme-one-dark";
import { useTheme } from "next-themes";

export function languageFor(path: string): Extension[] {
  const name = path.split("/").pop()?.toLowerCase() ?? "";
  const ext = name.includes(".") ? name.split(".").pop() : "";
  if (ext === "yml" || ext === "yaml") return [yaml()];
  if (ext === "json") return [json()];
  if (name === "caddyfile" || name.endsWith(".caddy") || ext === "conf") return [StreamLanguage.define(nginx)];
  if (name.startsWith("dockerfile") || ext === "dockerfile") return [StreamLanguage.define(dockerFile)];
  if (ext === "sh" || ext === "bash" || name === ".bashrc") return [StreamLanguage.define(shell)];
  if (ext === "toml") return [StreamLanguage.define(toml)];
  if (ext === "py") return [StreamLanguage.define(python)];
  if (ext === "js" || ext === "mjs" || ext === "ts") return [StreamLanguage.define(javascript)];
  if (name.startsWith(".env") || name.endsWith(".env") || ext === "ini" || ext === "properties" || ext === "service" || ext === "timer")
    return [StreamLanguage.define(properties)];
  return [];
}

const base = EditorView.theme({
  "&": { fontSize: "13px", height: "100%" },
  ".cm-scroller": { fontFamily: "var(--font-geist-mono), ui-monospace, monospace" },
});

export function CodeEditor({
  value,
  onChange,
  path,
  readOnly,
  height = "min(70vh, 48rem)",
}: {
  value: string;
  onChange?: (v: string) => void;
  path: string;
  readOnly?: boolean;
  height?: string;
}) {
  const { resolvedTheme } = useTheme();
  const extensions = useMemo(() => [...languageFor(path), base, EditorView.lineWrapping], [path]);
  return (
    <div className="overflow-hidden rounded-lg border">
      <CodeMirror
        value={value}
        onChange={onChange}
        readOnly={readOnly}
        editable={!readOnly}
        height={height}
        theme={resolvedTheme === "dark" ? oneDark : "light"}
        extensions={extensions}
        basicSetup={{ foldGutter: true, highlightActiveLine: !readOnly, autocompletion: false }}
      />
    </div>
  );
}

/** Side-by-side, read-only diff of `before` → `after`. */
export function DiffView({ before, after, path }: { before: string; after: string; path: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const { resolvedTheme } = useTheme();
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ext = [
      ...languageFor(path),
      base,
      EditorView.editable.of(false),
      EditorState.readOnly.of(true),
      ...(resolvedTheme === "dark" ? [oneDark] : []),
    ];
    const view = new MergeView({
      a: { doc: before, extensions: ext },
      b: { doc: after, extensions: ext },
      parent: el,
      collapseUnchanged: { margin: 3, minSize: 6 },
      gutter: true,
    });
    return () => view.destroy();
  }, [before, after, path, resolvedTheme]);
  return <div ref={ref} className="max-h-[60vh] overflow-auto rounded-lg border text-[13px]" />;
}
