import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { Camera, ChevronDown, Shield } from "lucide-react";
import { useState } from "react";

import {
  api,
  type Isolation,
  isolationLabel,
  projectIsolation,
  type Project,
  type ProjectIsolation,
  type Worktree,
} from "../lib/api";
import {
  keys,
  useIsolationSettings,
  useProjectDefaultIsolation,
  useSessions,
  useWorktreeIsolation,
} from "../lib/queries";
import { useUi } from "../lib/store";
import { useAsyncAction } from "../lib/useAsyncAction";
import { DoctorPanel } from "./DoctorPanel";
import { IsolationPicker } from "./IsolationPicker";
import { Button, Dialog, ErrorText, Field, WarnText } from "./ui";

/** Where this worktree's agents run, once the settings have loaded. */
export function useEffectiveIsolation(project: Project, worktree: Worktree): Isolation | undefined {
  const { data } = useWorktreeIsolation(project.id, worktree.path);
  const { data: settings } = useIsolationSettings();
  if (!data || !settings) return undefined;
  return data.choice.kind === "inherit" || project.remote ? projectIsolation(project, settings.default) : data.choice;
}

/** Which setting decides the worktree's isolation. */
function isolationSource(project: Project, choice: ProjectIsolation): string {
  if (choice.kind !== "inherit" && !project.remote) return "set for this worktree";
  if (project.isolation.kind !== "inherit" || project.remote) return `from project ${project.name}`;
  return "from the default in Settings › Isolation";
}

/** Header badge showing where this worktree's agents run; opens a dialog to change it. */
export function WorktreeIsolationBadge({ project, worktree }: { project: Project; worktree: Worktree }) {
  const [open, setOpen] = useState(false);
  const { data } = useWorktreeIsolation(project.id, worktree.path);
  const { data: settings } = useIsolationSettings();
  if (!data || !settings) return null;
  const effective =
    data.choice.kind === "inherit" || project.remote ? projectIsolation(project, settings.default) : data.choice;
  return (
    <>
      <button
        onClick={() => !project.remote && setOpen(true)}
        disabled={!!project.remote}
        title={
          project.remote
            ? `Remote project · ${isolationLabel(effective)}`
            : `${effective.kind === "none" ? "No isolation" : isolationLabel(effective)} · ${isolationSource(project, data.choice)} · click to change or save a snapshot`
        }
        className="flex h-7 max-w-40 items-center gap-1.5 rounded-md border border-line bg-elevated px-2 text-xs enabled:hover:bg-hover disabled:border-transparent disabled:bg-transparent disabled:text-muted"
      >
        <Shield size={13} className={clsx("shrink-0", effective.kind === "none" ? "text-faint" : "text-accent")} />
        <span className="truncate">{shortIsolationLabel(effective)}</span>
        {!project.remote && <ChevronDown size={12} className="shrink-0 text-faint" />}
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
  const openDialog = useUi((s) => s.openDialog);
  const [choice, setChoice] = useState<ProjectIsolation>(current);
  const { busy: saving, error, run } = useAsyncAction();
  const { data: sessions = [] } = useSessions();
  const projectDefault = useProjectDefaultIsolation(project);
  const effective = choice.kind === "inherit" ? projectDefault : choice;
  const running = sessions.filter((s) => s.worktreePath === worktree.path && s.state.kind === "running").length;
  const changed = JSON.stringify(choice) !== JSON.stringify(current);

  const save = () =>
    run(async () => {
      try {
        await api.setWorktreeIsolation(project.id, worktree.path, choice);
        onClose();
      } finally {
        await Promise.all([
          qc.invalidateQueries({ queryKey: keys.worktreeIsolation(worktree.path) }),
          qc.invalidateQueries({ queryKey: keys.sessions }),
        qc.invalidateQueries({ queryKey: ["environmentTemplates"] }),
        ]);
      }
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
      {!changed && isContainer(current.kind === "inherit" ? projectDefault : current) && (
        <div className="flex items-center gap-3 rounded-md border border-line p-3">
          <p className="flex-1 text-xs leading-relaxed text-muted">
            Save what's installed in this worktree's environment, so new worktrees start with it.
          </p>
          <Button
            onClick={() => {
              onClose();
              openDialog({ kind: "saveEnvironment", projectId: project.id, path: worktree.path });
            }}
          >
            <Camera size={13} />
            Save snapshot…
          </Button>
        </div>
      )}
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

export function isContainer(i: Isolation | ProjectIsolation | null | undefined): boolean {
  return i?.kind === "dockerSandbox" || i?.kind === "docker" || i?.kind === "podman";
}

function shortIsolationLabel(i: Isolation): string {
  switch (i.kind) {
    case "none":
      return "No isolation";
    case "macUser":
      return i.user;
    case "dockerSandbox":
      return "Sandbox";
    case "docker":
      return "Docker";
    case "podman":
      return "Podman";
    case "ssh":
      return i.host;
  }
}
