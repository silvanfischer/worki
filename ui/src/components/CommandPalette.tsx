import { useQueries } from "@tanstack/react-query";
import clsx from "clsx";
import { Bot, GitBranch, GitBranchPlus, GitCompareArrows, House, Moon, Search, Settings, SquareTerminal } from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";

import { api, worktreeLabel } from "../lib/api";
import { runCommand } from "../lib/commands";
import { keys, useAgentSettings, useProjects } from "../lib/queries";
import { useUi } from "../lib/store";

interface Item {
  id: string;
  label: string;
  detail?: string;
  icon: ReactNode;
  run: () => void;
}

export function CommandPalette({ onClose }: { onClose: () => void }) {
  const { data: projects = [] } = useProjects();
  const { data: agents } = useAgentSettings();
  const worktreeLists = useQueries({
    queries: projects.map((p) => ({ queryKey: keys.worktrees(p.id), queryFn: () => api.listWorktrees(p.id) })),
  });
  const select = useUi((s) => s.select);
  const openDialog = useUi((s) => s.openDialog);
  const toggleChanges = useUi((s) => s.toggleChanges);
  const toggleTerminal = useUi((s) => s.toggleTerminal);
  const theme = useUi((s) => s.theme);
  const setTheme = useUi((s) => s.setTheme);
  const selectedProjectId = useUi((s) => s.selectedProjectId);
  const selectedWorktree = useUi((s) => s.selectedWorktreePath);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const list = useRef<HTMLDivElement>(null);

  const items = useMemo<Item[]>(() => {
    const out: Item[] = [];
    if (selectedWorktree) {
      for (const p of agents?.profiles ?? []) {
        out.push({
          id: `agent:${p.id}`,
          label: `New ${p.name} session`,
          icon: <Bot size={14} />,
          run: () => runCommand({ kind: "newSession", profileId: p.id }),
        });
      }
      out.push({
        id: "shell",
        label: "New shell",
        icon: <SquareTerminal size={14} />,
        run: () => runCommand({ kind: "newSession", profileId: null }),
      });
      out.push({ id: "changes", label: "Toggle changes panel", detail: "⌘J", icon: <GitCompareArrows size={14} />, run: toggleChanges });
      out.push({ id: "terminal", label: "Toggle terminal", detail: "⌃`", icon: <SquareTerminal size={14} />, run: toggleTerminal });
    }
    if (selectedProjectId) {
      out.push({
        id: "newWorktree",
        label: "New worktree…",
        detail: "⌘⇧N",
        icon: <GitBranchPlus size={14} />,
        run: () => openDialog({ kind: "newWorktree", projectId: selectedProjectId }),
      });
    }
    out.push({
      id: "theme",
      label: theme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      icon: <Moon size={14} />,
      run: () => setTheme(theme === "dark" ? "light" : "dark"),
    });
    out.push({
      id: "settings",
      label: "Settings",
      detail: "⌘,",
      icon: <Settings size={14} />,
      run: () => openDialog({ kind: "settings", tab: "general" }),
    });
    projects.forEach((p, i) => {
      for (const w of worktreeLists[i]?.data ?? []) {
        out.push({
          id: `wt:${w.path}`,
          label: worktreeLabel(w),
          detail: p.name,
          icon: w.isMain ? <House size={14} /> : <GitBranch size={14} />,
          run: () => select(p.id, w.path),
        });
      }
    });
    return out;
  }, [agents, projects, worktreeLists, selectedWorktree, selectedProjectId, theme, toggleChanges, toggleTerminal, openDialog, setTheme, select]);

  const shown = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return items.filter((it) => {
      const text = `${it.label} ${it.detail ?? ""}`.toLowerCase();
      return words.every((w) => text.includes(w));
    });
  }, [items, query]);

  useEffect(() => setCursor(0), [query]);
  useEffect(() => {
    list.current?.querySelector(`[data-index="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  function choose(item: Item | undefined) {
    if (!item) return;
    onClose();
    item.run();
  }

  return (
    <div
      data-overlay
      className="fixed inset-0 z-50 flex justify-center bg-black/20 pt-[14vh]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="flex h-fit max-h-[60vh] w-[min(560px,90vw)] flex-col overflow-hidden rounded-xl border border-line bg-elevated shadow-2xl">
        <div className="flex items-center gap-2 border-b border-line px-3">
          <Search size={14} className="text-faint" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Jump to a worktree or run a command…"
            spellCheck={false}
            className="h-11 flex-1 bg-transparent text-[14px] outline-none placeholder:text-faint"
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              else if (e.key === "ArrowDown") {
                e.preventDefault();
                setCursor((c) => Math.min(shown.length - 1, c + 1));
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setCursor((c) => Math.max(0, c - 1));
              } else if (e.key === "Enter") {
                e.preventDefault();
                choose(shown[cursor]);
              }
            }}
          />
        </div>
        <div ref={list} className="overflow-y-auto p-1">
          {shown.length === 0 && <p className="px-3 py-4 text-xs text-faint">Nothing matches.</p>}
          {shown.map((it, i) => (
            <button
              key={it.id}
              data-index={i}
              onMouseMove={() => setCursor(i)}
              onClick={() => choose(it)}
              className={clsx(
                "flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13px]",
                i === cursor ? "bg-selected text-fg" : "text-muted",
              )}
            >
              <span className="text-faint">{it.icon}</span>
              <span className="min-w-0 flex-1 truncate">{it.label}</span>
              {it.detail && <span className="shrink-0 text-[11px] text-faint">{it.detail}</span>}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
