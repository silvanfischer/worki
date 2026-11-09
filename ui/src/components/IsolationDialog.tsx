import { useQueryClient } from "@tanstack/react-query";
import { ask } from "@tauri-apps/plugin-dialog";
import { LoaderCircle, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";

import { api, errorMessage, type Isolation, type LocalIsolation } from "../lib/api";
import { keys, useDoctor } from "../lib/queries";
import { AgentsPanel } from "./AgentsPanel";
import { DoctorPanel } from "./DoctorPanel";
import { IsolationPicker } from "./IsolationPicker";
import { Button, Dialog, ErrorText, Field } from "./ui";

export function IsolationDialog({ current, onClose }: { current: Isolation; onClose: () => void }) {
  const qc = useQueryClient();
  const [isolation, setIsolation] = useState<LocalIsolation>(current.kind === "ssh" ? { kind: "none" } : current);
  const [error, setError] = useState<string | null>(null);
  const valid = isolation.kind !== "macUser" || /^[A-Za-z0-9_][A-Za-z0-9_.-]{0,30}$/.test(isolation.user);
  const { data: checks } = useDoctor(valid ? isolation : null, null);
  const ready = isolation.kind === "none" || (!!checks?.length && !checks.some((c) => c.fix === "provision"));
  const userExists = isolation.kind === "macUser" && !!checks?.find((c) => c.id === "user")?.ok;
  const [removing, setRemoving] = useState(false);

  async function removeUser() {
    if (isolation.kind !== "macUser") return;
    const ok = await ask(
      `Remove the agent user “${isolation.user}”?\n\nThis stops its sessions, removes all file permissions worki gave it, and deletes the account and its home folder (/Users/${isolation.user}), including agents installed there and copied config. Your worktrees are not touched.\n\nmacOS only allows deleting accounts from Terminal, so worki opens a Terminal window: enter your password there. worki finishes once the account is gone.`,
      { title: "Remove agent user", kind: "warning", okLabel: "Remove user" },
    );
    if (!ok) return;
    setRemoving(true);
    setError(null);
    try {
      await api.removeIsolation(isolation);
      await Promise.all([
        qc.invalidateQueries({ queryKey: keys.isolationSettings }),
        qc.invalidateQueries({ queryKey: keys.projects }),
        qc.invalidateQueries({ queryKey: keys.sessions }),
        qc.invalidateQueries({ queryKey: keys.allDoctor }),
      ]);
      setIsolation({ kind: "none" });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setRemoving(false);
    }
  }

  const pending = JSON.stringify(isolation) !== JSON.stringify(current);
  useEffect(() => {
    if (!pending || !valid || !ready || removing) return;
    const timer = setTimeout(async () => {
      try {
        await api.saveIsolationSettings({ default: isolation });
        setError(null);
        await qc.invalidateQueries({ queryKey: keys.isolationSettings });
      } catch (e) {
        setError(errorMessage(e));
      }
    }, 400);
    return () => clearTimeout(timer);
  }, [isolation, pending, valid, ready, removing, qc]);

  return (
    <Dialog
      title="Isolation"
      onClose={onClose}
      width={560}
      footer={
        userExists && (
          <Button variant="ghost" className="mr-auto text-removed" onClick={removeUser} disabled={removing}>
            {removing ? <LoaderCircle size={13} className="animate-spin" /> : <Trash2 size={13} />}
            {removing ? "Waiting for Terminal…" : "Remove user…"}
          </Button>
        )
      }
    >
      <p className="text-xs leading-relaxed text-muted">
        Choose where agents run by default. Projects can override this in their settings. Setup scripts run in the same
        place as the agents.
      </p>
      <Field label="Default for all projects">
        <IsolationPicker value={isolation} onChange={setIsolation} />
      </Field>
      {valid ? (
        <>
          <DoctorPanel isolation={isolation} projectId={null} />
          {ready && <AgentsPanel isolation={isolation} />}
        </>
      ) : (
        <ErrorText>Use letters, digits, “_”, “.” or “-” (max 31 characters).</ErrorText>
      )}
      {!ready && valid && <p className="text-xs text-faint">Finish the setup above; the default changes once it's done.</p>}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
