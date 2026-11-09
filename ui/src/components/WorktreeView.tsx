import { revealItemInDir } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import { useState } from "react";
import { ArrowDown, ArrowUp, Copy, FolderOpen, GitBranch, PanelRight, Trash2 } from "lucide-react";

import { type ChangedFile, type Project, type Worktree, worktreeLabel } from "../lib/api";
import { useWorktreeStatus } from "../lib/queries";
import { useUi } from "../lib/store";
import { DiffView } from "./DiffView";
import { OpenInMenu } from "./OpenInMenu";
import { ResizeHandle } from "./ResizeHandle";
import { SessionArea } from "./SessionArea";
import { DiffStat, IconButton } from "./ui";
import { WorktreeIsolationBadge } from "./WorktreeIsolation";

export function WorktreeView({ project, worktree }: { project: Project; worktree: Worktree }) {
  const openDialog = useUi((s) => s.openDialog);
  const showChanges = useUi((s) => s.showChanges);
  const toggleChanges = useUi((s) => s.toggleChanges);
  const { data: status, error } = useWorktreeStatus(worktree.path, 5_000);
  const [diffFile, setDiffFile] = useState<string | null>(null);
  const widths = useUi((s) => s.panelWidths);

  return (
    <div className="flex h-full flex-col">
      <header data-tauri-drag-region className="flex h-11 shrink-0 items-center gap-3 border-b border-line px-4">
        <GitBranch size={15} className="text-muted" />
        <div data-tauri-drag-region className="flex min-w-0 flex-1 items-baseline gap-2">
          <h1 className="truncate text-[14px] font-semibold">{worktreeLabel(worktree)}</h1>
          <span className="truncate text-xs text-faint">{project.name}</span>
        </div>
        {status && (status.ahead > 0 || status.behind > 0) && (
          <span className="flex items-center gap-1.5 font-mono text-xs text-muted" title={`vs ${status.upstream}`}>
            <ArrowUp size={12} />
            {status.ahead}
            <ArrowDown size={12} />
            {status.behind}
          </span>
        )}
        <WorktreeIsolationBadge project={project} worktree={worktree} />
        <OpenInMenu path={worktree.path} />
        <IconButton label="Copy path" onClick={() => navigator.clipboard.writeText(worktree.path)}>
          <Copy size={14} />
        </IconButton>
        <IconButton label="Reveal in Finder" onClick={() => revealItemInDir(worktree.path)}>
          <FolderOpen size={14} />
        </IconButton>
        {!worktree.isMain && (
          <IconButton
            label="Remove worktree"
            onClick={() => openDialog({ kind: "removeWorktree", projectId: project.id, path: worktree.path })}
          >
            <Trash2 size={14} />
          </IconButton>
        )}
        <IconButton
          label={showChanges ? "Hide changes" : "Show changes"}
          onClick={toggleChanges}
          className={clsx(showChanges && "text-fg")}
        >
          <PanelRight size={14} />
        </IconButton>
      </header>

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <section className="min-h-0 min-w-[240px] flex-1">
          <SessionArea project={project} worktree={worktree} />
        </section>

        {diffFile && (
          <section className="relative min-h-0 shrink-0 border-l border-line" style={{ width: widths.diff }}>
            <ResizeHandle panel="diff" edge="left" />
            <DiffView worktreePath={worktree.path} file={diffFile} onClose={() => setDiffFile(null)} />
          </section>
        )}

        {showChanges && (
          <aside className="relative flex min-h-0 shrink-0 flex-col border-l border-line" style={{ width: widths.changes }}>
            <ResizeHandle panel="changes" edge="left" />
            <div className="flex h-9 shrink-0 items-center justify-between px-3">
              <span className="text-[11px] font-semibold tracking-wide text-faint uppercase">Changes</span>
              {status && <DiffStat insertions={status.insertions} deletions={status.deletions} />}
            </div>
            <div className="selectable flex-1 overflow-y-auto px-1.5 pb-3">
              {error && <p className="px-2 text-xs text-removed">{String(error)}</p>}
              {status?.changedFiles.length === 0 && <p className="px-2 py-1 text-xs text-faint">Working tree clean</p>}
              {status?.changedFiles.map((f) => (
                <ChangeRow
                  key={f.path}
                  file={f}
                  selected={f.path === diffFile}
                  onOpen={() => setDiffFile(f.path === diffFile ? null : f.path)}
                />
              ))}
            </div>
          </aside>
        )}
      </div>
    </div>
  );
}

function ChangeRow({ file, selected, onOpen }: { file: ChangedFile; selected: boolean; onOpen: () => void }) {
  const kind = changeKind(file.code);
  const slash = file.path.lastIndexOf("/");
  return (
    <div
      onClick={onOpen}
      className={clsx("flex h-6 items-center gap-2 rounded px-1.5", selected ? "bg-selected" : "hover:bg-hover")}
      title={`${file.path} — click to view the diff`}
    >
      <span
        className={clsx(
          "w-3 text-center font-mono text-[11px] font-semibold",
          kind === "A" || kind === "U" ? "text-added" : kind === "D" ? "text-removed" : "text-warn",
        )}
      >
        {kind}
      </span>
      <span className="min-w-0 truncate text-xs">
        {file.path.slice(slash + 1)}
        {slash > 0 && <span className="ml-1.5 text-faint">{file.path.slice(0, slash)}</span>}
      </span>
    </div>
  );
}

function changeKind(code: string): string {
  if (code === "??") return "U";
  const c = code.trim()[0] ?? "M";
  return c === "R" || c === "C" ? "R" : c;
}
