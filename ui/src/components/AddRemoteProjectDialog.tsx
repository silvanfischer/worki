import { useQueryClient } from "@tanstack/react-query";
import { LoaderCircle } from "lucide-react";
import { useState } from "react";

import { api } from "../lib/api";
import { keys } from "../lib/queries";
import { useUi } from "../lib/store";
import { useAsyncAction } from "../lib/useAsyncAction";
import { parseRemote, RemoteFields, remoteForm } from "./RemoteFields";
import { Button, Dialog, ErrorText, Field, Input } from "./ui";

export function AddRemoteProjectDialog({ onClose }: { onClose: () => void }) {
  const qc = useQueryClient();
  const select = useUi((s) => s.select);
  const [form, setForm] = useState(() => remoteForm(null));
  const [path, setPath] = useState("");
  const { busy: adding, error, setError, run } = useAsyncAction();

  function add() {
    const remote = parseRemote(form);
    if (typeof remote === "string") return setError(remote);
    run(async () => {
      const project = await api.addRemoteProject(remote, path.trim(), form.password || null);
      await qc.invalidateQueries({ queryKey: keys.projects });
      select(project.id, project.repoPath);
      onClose();
    });
  }

  return (
    <Dialog
      title="Add remote repository"
      onClose={onClose}
      width={500}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={add} disabled={adding || !form.host.trim() || !path.trim()}>
            {adding && <LoaderCircle size={13} className="animate-spin" />}
            {adding ? "Connecting…" : "Add"}
          </Button>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-muted">
        The repository stays on the other machine: worki creates worktrees there and runs agents there over SSH.
      </p>
      <RemoteFields value={form} onChange={setForm} />
      <Field label="Repository path on the host">
        <Input
          value={path}
          onChange={(e) => setPath(e.target.value)}
          placeholder="~/src/my-app"
          className="font-mono text-xs"
        />
      </Field>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
