import { useQueryClient } from "@tanstack/react-query";
import { Shield } from "lucide-react";
import { useState } from "react";

import { api, isolationLabel, type Project, type ProjectIsolation, type Worktree } from "../lib/api";
import { keys, useProjectDefaultIsolation, useSessions, useWorktreeIsolation } from "../lib/queries";
import { useAsyncAction } from "../lib/useAsyncAction";
import { DoctorPanel } from "./DoctorPanel";
import { IsolationPicker } from "./IsolationPicker";
import { Button, Dialog, ErrorText, Field, WarnText } from "./ui";

/** Header badge showing where this worktree's agents run; opens a dialog to change it. */
export function WorktreeIsolationBadge({ project, worktree }: { project: Project; worktree: Worktree }) {
  const [open, setOpen] = useState(false);
  const { data } = useWorktreeIsolation(project.id, worktree.path);
  if (!data) return null;
  return (
    <>
      <button
        onClick={() => !project.remote && setOpen(true)}
        title={project.remote ? "Remote project" : "Change this worktree's isolation"}
        className="flex h-7 max-w-52 items-center gap-1.5 rounded-md px-2 text-xs text-muted hover:bg-hover hover:text-fg"
      >
        <Shield size={13} className={data.effective.kind === "none" ? "text-faint" : "text-accent"} />
        <span className="truncate">{data.effective.kind === "none" ? "No isolation" : isolationLabel(data.effective)}</span>
      </button>
      {open && <WorktreeIsolationDialog project={project} worktree={worktree} current={data.choice} onClose={() => setOpen(false)} />}
    </>
  );
}

function WorktreeIsolationDialog({
  project,
  worktree,
  current,
  onClose,
}: {
  project: Project;
  worktree: Worktree;
  current: ProjectIsolation;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const [choice, setChoice] = useState<ProjectIsolation>(current);
  const { busy: saving, error, run } = useAsyncAction();
  const { data: sessions = [] } = useSessions();
  const projectDefault = useProjectDefaultIsolation(project);
  const effective = choice.kind === "inherit" ? projectDefault : choice;
  const running = sessions.filter((s) => s.worktreePath === worktree.path && s.state.kind === "running").length;
  const changed = JSON.stringify(choice) !== JSON.stringify(current);

  const save = () =>
    run(async () => {
      await api.setWorktreeIsolation(project.id, worktree.path, choice);
      await Promise.all([
        qc.invalidateQueries({ queryKey: keys.worktreeIsolation(worktree.path) }),
        qc.invalidateQueries({ queryKey: keys.sessions }),
      ]);
      onClose();
    });

  return (
    <Dialog
      title={`Isolation · ${worktree.branch ?? worktree.path.split("/").pop()}`}
      onClose={onClose}
      width={540}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save} disabled={!changed || saving}>
            {saving ? "Applying…" : "Apply"}
          </Button>
        </>
      }
    >
      <Field label="Where agents in this worktree run">
        <IsolationPicker
          value={choice}
          onChange={setChoice}
          allowInherit
          inheritName="Project default"
          inheritLabel={projectDefault ? isolationLabel(projectDefault) : undefined}
        />
      </Field>
      {changed && (
        <WarnText>
          {running > 0 ? `${running} running session${running === 1 ? "" : "s"} will be closed. ` : ""}
          The worktree's current environment (its container or sandbox, if any) is removed; the next session sets up the
          new one.
        </WarnText>
      )}
      {effective && effective.kind !== "none" && effective.kind !== "ssh" && (
        <DoctorPanel key={JSON.stringify(effective)} isolation={effective} projectId={project.id} />
      )}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
