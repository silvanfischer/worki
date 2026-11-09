import { useQueryClient } from "@tanstack/react-query";
import { Pencil, Plus, Server, Trash2 } from "lucide-react";
import { useState } from "react";

import { api, errorMessage, remoteLabel, type SshHost } from "../lib/api";
import { keys, useProjects, useSshHosts, useSshPasswordSet } from "../lib/queries";
import { ConnectionFields, connectionForm, parseConnection } from "./RemoteFields";
import { Button, ErrorText, Field, IconButton, Input } from "./ui";

/** Saved SSH connections that remote projects can share. */
export function SshHostsSettings() {
  const { data: hosts = [] } = useSshHosts();
  const { data: projects = [] } = useProjects();
  const [editing, setEditing] = useState<string | null>(null);

  return (
    <section className="flex min-h-0 flex-1 flex-col">
      <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">SSH hosts</h2>
      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-6 pb-6">
        <p className="text-xs leading-relaxed text-faint">
          Connections you can pick when adding a remote repository. Projects using a host follow its changes, including
          its password.
        </p>
        {hosts.length === 0 && editing !== "" && <p className="text-xs text-muted">No saved hosts yet.</p>}
        <div className="flex flex-col gap-1">
          {hosts.map((h) => {
            const users = projects.filter((p) => p.remote?.hostId === h.id);
            return editing === h.id ? (
              <HostEditor key={h.id} host={h} onDone={() => setEditing(null)} />
            ) : (
              <div key={h.id} className="group flex items-center gap-3 rounded-md px-2 py-1.5 hover:bg-hover">
                <Server size={14} className="text-muted" />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13px]">{h.name}</div>
                  <div className="truncate font-mono text-[11px] text-faint">{remoteLabel(h)}</div>
                </div>
                <span
                  className="text-[11px] text-faint"
                  title={users.map((p) => p.name).join(", ") || undefined}
                >
                  {users.length === 0 ? "Not used" : `${users.length} project${users.length === 1 ? "" : "s"}`}
                </span>
                <IconButton label="Edit host" onClick={() => setEditing(h.id)}>
                  <Pencil size={13} />
                </IconButton>
              </div>
            );
          })}
        </div>
        {editing === "" ? (
          <HostEditor host={null} onDone={() => setEditing(null)} />
        ) : (
          <Button className="self-start" onClick={() => setEditing("")}>
            <Plus size={13} />
            Add host
          </Button>
        )}
      </div>
    </section>
  );
}

function HostEditor({ host, onDone }: { host: SshHost | null; onDone: () => void }) {
  const qc = useQueryClient();
  const [name, setName] = useState(host?.name ?? "");
  const [form, setForm] = useState(() => connectionForm(host));
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const { data: passwordStored = false, refetch } = useSshPasswordSet(host);

  async function run(action: () => Promise<unknown>) {
    setError(null);
    setBusy(true);
    try {
      await action();
      await Promise.all([
        qc.invalidateQueries({ queryKey: keys.sshHosts }),
        qc.invalidateQueries({ queryKey: keys.projects }),
      ]);
      onDone();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  }

  function save() {
    const conn = parseConnection(form);
    if (typeof conn === "string") return setError(conn);
    run(() => api.saveSshHost({ id: host?.id ?? "", name: name.trim() || conn.host, ...conn }, form.password || null));
  }

  return (
    <div className="flex flex-col gap-3 rounded-lg border border-line bg-panel p-3">
      <Field label="Name">
        <Input value={name} onChange={(e) => setName(e.target.value)} placeholder={form.host || "devbox"} autoFocus />
      </Field>
      <ConnectionFields
        value={form}
        onChange={setForm}
        passwordStored={passwordStored}
        onRemovePassword={
          host
            ? async () => {
                await api.setSshPassword(host, null).catch((e) => setError(errorMessage(e)));
                refetch();
              }
            : undefined
        }
      />
      <ErrorText>{error}</ErrorText>
      <div className="flex items-center gap-2">
        {host && (
          <Button
            variant="ghost"
            disabled={busy}
            onClick={() => run(() => api.deleteSshHost(host.id))}
            title="Projects using this host keep its settings as their own"
          >
            <Trash2 size={12} />
            Delete
          </Button>
        )}
        <span className="flex-1" />
        <Button variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button variant="primary" onClick={save} disabled={busy || !form.host.trim()}>
          {host ? "Save" : "Add host"}
        </Button>
      </div>
    </div>
  );
}
