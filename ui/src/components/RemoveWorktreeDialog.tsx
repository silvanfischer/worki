import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { api, type Project, type Worktree, worktreeLabel } from "../lib/api";
import { keys, useWorktrees, useWorktreeStatus } from "../lib/queries";
import { useUi } from "../lib/store";
import { useAsyncAction } from "../lib/useAsyncAction";
import { Button, Checkbox, Dialog, ErrorText, WarnText } from "./ui";

export function RemoveWorktreeDialog({
  project,
  path,
  onClose,
}: {
  project: Project;
  path: string;
  onClose: () => void;
}) {
  const { data: worktrees } = useWorktrees(project.id);
  const worktree = worktrees?.find((w) => w.path === path);
  return worktree ? <RemoveWorktreeForm project={project} worktree={worktree} onClose={onClose} /> : null;
}

function RemoveWorktreeForm({
  project,
  worktree,
  onClose,
}: {
  project: Project;
  worktree: Worktree;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { selectedWorktreePath, select } = useUi();
  const { data: status } = useWorktreeStatus(worktree.path);
  const dirty = status?.changedFiles.length ?? 0;

  const [deleteBranch, setDeleteBranch] = useState(false);
  const [force, setForce] = useState(false);
  const { busy, error, run } = useAsyncAction();

  const remove = () =>
    run(async () => {
      try {
        await api.removeWorktree(project.id, {
          path: worktree.path,
          force,
          deleteBranch: deleteBranch && !!worktree.branch,
        });
        if (selectedWorktreePath === worktree.path) select(project.id, project.repoPath);
        onClose();
      } finally {
        qc.invalidateQueries({ queryKey: keys.worktrees(project.id) });
        qc.invalidateQueries({ queryKey: keys.branches(project.id) });
      }
    });

  return (
    <Dialog
      title="Remove worktree"
      onClose={onClose}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" onClick={remove} disabled={busy || (dirty > 0 && !force)}>
            {busy ? "Removing…" : "Remove"}
          </Button>
        </>
      }
    >
      <p className="leading-relaxed">
        Remove <span className="font-semibold">{worktreeLabel(worktree)}</span> from{" "}
        <span className="font-semibold">{project.name}</span>?
        <span className="selectable mt-1 block font-mono text-xs text-muted">{worktree.path}</span>
      </p>
      {dirty > 0 && (
        <WarnText>
          This worktree has {dirty} uncommitted {dirty === 1 ? "change" : "changes"} that will be lost.
        </WarnText>
      )}
      {worktree.branch && (
        <Checkbox checked={deleteBranch} onChange={setDeleteBranch}>
          Also delete branch <span className="font-mono text-xs">{worktree.branch}</span>
        </Checkbox>
      )}
      <Checkbox checked={force} onChange={setForce}>
        <span className="flex flex-col gap-0.5">
          <span>Force</span>
          <span className="text-xs text-faint">Discard uncommitted changes and delete unmerged branches.</span>
        </span>
      </Checkbox>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
