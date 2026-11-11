import { useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";

import { api, errorMessage, isolationLabel, type LogLine, type Project, type ProjectIsolation } from "../lib/api";
import { keys, useBranches, useProjectDefaultIsolation, useSetupScripts } from "../lib/queries";
import { useUi } from "../lib/store";
import { BranchPicker } from "./BranchPicker";
import { IsolationPicker } from "./IsolationPicker";
import { Button, Checkbox, Dialog, ErrorText, Field, Input, LogView } from "./ui";

type Phase = "edit" | "running" | "setupFailed";

export function NewWorktreeDialog({ project, onClose }: { project: Project; onClose: () => void }) {
  const qc = useQueryClient();
  const select = useUi((s) => s.select);
  const { data: branches = [] } = useBranches(project.id);
  const { data: scripts } = useSetupScripts(project.id);
  const setupScript = scripts?.project?.trim() || scripts?.repoFile?.trim() || null;

  const [name, setName] = useState("");
  const [base, setBase] = useState(project.defaultBase);
  const [fetching, setFetching] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);

  async function fetchLatest() {
    setFetching(true);
    setFetchError(null);
    try {
      await api.fetchRemote(project.repoPath);
      await qc.invalidateQueries({ queryKey: keys.branches(project.id) });
    } catch (e) {
      setFetchError(errorMessage(e));
    } finally {
      setFetching(false);
    }
  }
  const [runSetup, setRunSetup] = useState(true);
  const [isolation, setIsolation] = useState<ProjectIsolation>({ kind: "inherit" });
  const projectDefault = useProjectDefaultIsolation(project);
  const [phase, setPhase] = useState<Phase>("edit");
  const [error, setError] = useState<string | null>(null);
  const [log, setLog] = useState<LogLine[]>([]);

  const branch = branchName(name);
  const nameTaken = branches.includes(branch);
  const localBranches = branches.filter((b) => !b.startsWith("origin/"));
  const canSubmit = !!branch && !nameTaken && !!base && phase === "edit";

  useEffect(() => {
    if (branches.length > 0 && !branches.includes(base)) setBase(localBranches[0] ?? branches[0]);
  }, [branches]);

  async function submit() {
    if (!canSubmit) return;
    setPhase("running");
    setError(null);
    setLog([]);
    try {
      const res = await api.createWorktree(
        project.id,
        { name, base, runSetup: runSetup && !!setupScript, isolation },
        (line) => setLog((l) => [...l, line]),
      );
      await qc.invalidateQueries({ queryKey: keys.worktrees(project.id) });
      qc.invalidateQueries({ queryKey: keys.branches(project.id) });
      select(project.id, res.worktree.path);
      if (res.setupError) {
        setError(`Worktree created, but the setup script failed: ${res.setupError}`);
        setPhase("setupFailed");
      } else {
        onClose();
      }
    } catch (e) {
      setError(errorMessage(e));
      setPhase("edit");
    }
  }

  return (
    <Dialog
      title={`New worktree · ${project.name}`}
      onClose={phase === "running" ? () => {} : onClose}
      width={520}
      footer={
        phase === "setupFailed" ? (
          <Button variant="primary" onClick={onClose}>
            Close
          </Button>
        ) : (
          <>
            <Button variant="ghost" onClick={onClose} disabled={phase === "running"}>
              Cancel
            </Button>
            <Button variant="primary" onClick={submit} disabled={!canSubmit}>
              {phase === "running" ? "Creating…" : "Create worktree"}
            </Button>
          </>
        )
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        <Field label="Start from" hint={fetchError && <span className="text-removed">Couldn't fetch: {fetchError}</span>}>
          <div className="flex gap-2">
            <div className="min-w-0 flex-1">
              <BranchPicker
                value={base}
                branches={branches.length > 0 ? branches : [base]}
                onChange={setBase}
                disabled={phase !== "edit"}
              />
            </div>
            <Button
              type="button"
              onClick={fetchLatest}
              disabled={fetching || phase !== "edit"}
              title="Fetch the latest branches from the remote"
              className="h-8"
            >
              <RefreshCw size={12} className={fetching ? "animate-spin" : undefined} />
              {fetching ? "Fetching…" : "Fetch"}
            </Button>
          </div>
        </Field>
        <Field
          label="Worktree name"
          hint={
            nameTaken ? (
              <span className="text-removed">A branch named “{branch}” already exists.</span>
            ) : branch ? (
              <span className="flex flex-col gap-0.5">
                <span>
                  New branch <span className="font-mono text-muted">{branch}</span> from{" "}
                  <span className="font-mono text-muted">{base}</span>
                </span>
                <span className="truncate">
                  Folder <span className="font-mono text-muted">{tildify(`${project.worktreeRoot}/${slug(branch)}`)}</span>
                </span>
              </span>
            ) : (
              "Also used as the branch name."
            )
          }
        >
          <Input
            autoFocus
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="feature/my-change"
            disabled={phase !== "edit"}
          />
        </Field>
        {!project.remote && (
          <Field label="Isolation">
            <IsolationPicker
              value={isolation}
              onChange={setIsolation}
              allowInherit
              inheritName="Project default"
              inheritLabel={projectDefault ? isolationLabel(projectDefault) : undefined}
            />
          </Field>
        )}
        <Checkbox checked={runSetup && !!setupScript} onChange={setRunSetup} disabled={!setupScript || phase !== "edit"}>
          <span className="flex flex-col gap-0.5">
            <span>Run setup script</span>
            <span className="text-xs text-faint">
              {setupScript
                ? scripts?.project?.trim()
                  ? "From project settings"
                  : "From .worki.toml"
                : "No setup script configured"}
            </span>
          </span>
        </Checkbox>
        <button type="submit" hidden />
      </form>

      {log.length > 0 && <LogView lines={log} className="max-h-56 bg-panel" />}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

/** Mirrors `paths::branch_name` in worki-core. */
function branchName(name: string): string {
  return name.trim().split(/\s+/).filter(Boolean).join("-");
}

/** Mirrors `paths::slug` in worki-core. */
function slug(name: string): string {
  const s = name
    .toLowerCase()
    .replace(/[^a-z0-9._]+/g, "-")
    .replace(/^[-.]+|[-.]+$/g, "");
  return s || "worktree";
}

function tildify(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, "~");
}
