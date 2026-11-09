import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Copy, FolderOpen, Trash2 } from "lucide-react";

import type { Project, Worktree } from "../lib/api";
import type { DialogState } from "../lib/store";

/** The worktree's secondary actions, shared by the header's and the sidebar's menus. */
export function worktreeActions(
  project: Project,
  worktree: Worktree,
  openDialog: (d: DialogState) => void,
) {
  return [
    ...(project.remote
      ? []
      : [{ label: "Reveal in Finder", icon: <FolderOpen size={13} />, onSelect: () => revealItemInDir(worktree.path) }]),
    { label: "Copy path", icon: <Copy size={13} />, onSelect: () => navigator.clipboard.writeText(worktree.path) },
    ...(worktree.isMain
      ? []
      : [
          {
            label: "Remove worktree…",
            icon: <Trash2 size={13} />,
            danger: true,
            onSelect: () => openDialog({ kind: "removeWorktree", projectId: project.id, path: worktree.path }),
          },
        ]),
  ];
}
