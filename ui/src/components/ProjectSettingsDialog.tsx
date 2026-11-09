import { useQueryClient } from "@tanstack/react-query";
import { ask, message } from "@tauri-apps/plugin-dialog";
import { useState } from "react";

import {
  api,
  errorMessage,
  type Isolation,
  isolationLabel,
  type Project,
  type ProjectIsolation,
  projectIsolation,
} from "../lib/api";
import { keys, useIsolationSettings, useSetupScripts, useSshPasswordSet } from "../lib/queries";
import { useUi } from "../lib/store";
import { AgentsPanel } from "./AgentsPanel";
import { DoctorPanel } from "./DoctorPanel";
import { IsolationPicker } from "./IsolationPicker";
import { parseRemote, RemoteFields, remoteForm } from "./RemoteFields";
import { Button, Dialog, ErrorText, Field, Input, Textarea } from "./ui";

export function ProjectSettingsDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const qc = useQueryClient();
  const { selectedProjectId, select } = useUi();
  const { data: scripts } = useSetupScripts(project.id);
  const { data: isolationSettings } = useIsolationSettings();
  const globalIsolation: Isolation = isolationSettings?.default ?? { kind: "none" };

  const [name, setName] = useState(project.name);
  const [worktreeRoot, setWorktreeRoot] = useState(project.worktreeRoot);
  const [defaultBase, setDefaultBase] = useState(project.defaultBase);
  const [setupScript, setSetupScript] = useState(project.setupScript ?? "");
  const [isolation, setIsolation] = useState<ProjectIsolation>(project.isolation);
  const [form, setForm] = useState(() => remoteForm(project.remote));
  const parsed = project.remote ? parseRemote(form) : null;
  const remote = parsed && typeof parsed !== "string" ? parsed : project.remote;
  const { data: passwordStored = false, refetch: refetchPassword } = useSshPasswordSet(project.remote);
  const [error, setError] = useState<string | null>(null);
  const effective = projectIsolation({ ...project, isolation, remote }, globalIsolation);

  async function save() {
    setError(null);
    if (isolation.kind === "macUser" && !isolation.user) return setError("Enter a user name.");
    try {
      if (typeof parsed === "string") return setError(parsed);
      await api.updateProject(project.id, {
        name,
        worktreeRoot,
        defaultBase,
        setupScript,
        isolation,
        ...(parsed ? { remote: parsed } : {}),
      });
      if (parsed && form.password) await api.setSshPassword(parsed, form.password);
      await qc.invalidateQueries({ queryKey: keys.projects });
      qc.invalidateQueries({ queryKey: keys.setupScripts(project.id) });
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function removeProject() {
    const ok = await ask(
      `Remove “${project.name}” from worki? Its sessions are closed and its isolation is cleaned up (agent containers or sandboxes deleted, agent-user access removed). The repository and its worktrees stay on disk.`,
      { title: "Remove project", kind: "warning", okLabel: "Remove" },
    );
    if (!ok) return;
    try {
      const warning = await api.removeProject(project.id);
      if (warning) await message(warning, { title: "Project removed", kind: "warning" });
      if (selectedProjectId === project.id) select("", null);
      await qc.invalidateQueries({ queryKey: keys.projects });
      onClose();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <Dialog
      title="Project settings"
      onClose={onClose}
      width={540}
      footer={
        <>
          <Button variant="ghost" className="mr-auto text-removed" onClick={removeProject}>
            Remove project…
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save}>
            Save
          </Button>
        </>
      }
    >
      <Field label="Name">
        <Input value={name} onChange={(e) => setName(e.target.value)} />
      </Field>
      <Field label={remote ? "Repository on the host" : "Repository"}>
        <Input value={project.repoPath} readOnly className="text-muted" />
      </Field>
      {project.remote && (
        <RemoteFields
          value={form}
          onChange={setForm}
          passwordStored={passwordStored}
          onRemovePassword={async () => {
            await api.setSshPassword(project.remote!, null).catch((e) => setError(errorMessage(e)));
            refetchPassword();
          }}
        />
      )}
      <div className="grid grid-cols-[1fr_160px] gap-3">
        <Field label="Worktree folder" hint="New worktrees are created in <folder>/<branch>.">
          <Input value={worktreeRoot} onChange={(e) => setWorktreeRoot(e.target.value)} />
        </Field>
        <Field label="Default base">
          <Input value={defaultBase} onChange={(e) => setDefaultBase(e.target.value)} />
        </Field>
      </div>
      <Field
        label="Setup script"
        hint={
          scripts?.repoFile
            ? setupScript.trim()
              ? "Overrides the script in .worki.toml."
              : "Empty — the script from .worki.toml is used."
            : "Runs with zsh in each new worktree. $WORKI_ROOT_PATH, $WORKI_WORKTREE_PATH and $WORKI_BRANCH are set."
        }
      >
        <Textarea
          rows={5}
          value={setupScript}
          onChange={(e) => setSetupScript(e.target.value)}
          placeholder={scripts?.repoFile ?? 'cp "$WORKI_ROOT_PATH/.env" .env\npnpm install'}
        />
      </Field>
      {!remote && (
        <Field label="Default isolation for new worktrees">
          <IsolationPicker
            value={isolation}
            onChange={setIsolation}
            allowInherit
            inheritLabel={isolationLabel(globalIsolation)}
          />
        </Field>
      )}
      {effective.kind !== "none" && (
        <DoctorPanel key={JSON.stringify(effective)} isolation={effective} projectId={project.id} />
      )}
      {project.remote && <AgentsPanel key={JSON.stringify(project.remote)} isolation={{ kind: "ssh", ...project.remote }} />}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
