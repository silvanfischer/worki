import { useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { api, errorMessage, type Remote, remoteLabel, type SshHost } from "../lib/api";
import { keys, useSshHosts, useSshPasswordSet } from "../lib/queries";
import { Checkbox, ErrorText, Field, Input, Select } from "./ui";

export interface ConnectionForm {
  host: string;
  user: string;
  port: string;
  password: string;
}

interface RemoteForm extends ConnectionForm {
  keepAlive: boolean;
  hostId: string | null;
}

export function remoteForm(r: Remote | null): RemoteForm {
  return {
    host: r?.host ?? "",
    user: r?.user ?? "",
    port: r?.port ? String(r.port) : "",
    password: "",
    keepAlive: r?.keepAlive ?? true,
    hostId: r?.hostId ?? null,
  };
}

export function connectionForm(h: SshHost | null): ConnectionForm {
  return { host: h?.host ?? "", user: h?.user ?? "", port: h?.port ? String(h.port) : "", password: "" };
}

/** Host, user and port, or an error message when they're not valid. */
export function parseConnection(f: ConnectionForm): Pick<Remote, "host" | "user" | "port"> | string {
  const host = f.host.trim();
  if (!host) return "Enter a host.";
  const port = f.port.trim() ? Number(f.port.trim()) : null;
  if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535)) return "The port must be a number from 1 to 65535.";
  return { host, user: f.user.trim() || null, port };
}

/** The connection settings, or an error message when they're not valid. */
export function parseRemote(f: RemoteForm): Remote | string {
  const conn = parseConnection(f);
  if (typeof conn === "string") return conn;
  return { ...conn, keepAlive: f.keepAlive, hostId: f.hostId };
}

export function ConnectionFields({
  value,
  onChange,
  passwordStored,
  onRemovePassword,
}: {
  value: ConnectionForm;
  onChange: (v: ConnectionForm) => void;
  passwordStored?: boolean;
  onRemovePassword?: () => void;
}) {
  const set = (patch: Partial<ConnectionForm>) => onChange({ ...value, ...patch });
  return (
    <>
      <div className="grid grid-cols-[1fr_96px] gap-3">
        <Field label="Host" hint="Host name, IP address or an alias from ~/.ssh/config.">
          <Input value={value.host} onChange={(e) => set({ host: e.target.value })} placeholder="devbox.example.com" />
        </Field>
        <Field label="Port">
          <Input value={value.port} onChange={(e) => set({ port: e.target.value })} placeholder="22" inputMode="numeric" />
        </Field>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <Field label="User name" hint="Empty: from ~/.ssh/config or your Mac user name.">
          <Input value={value.user} onChange={(e) => set({ user: e.target.value })} placeholder="optional" />
        </Field>
        <Field
          label="Password"
          hint={
            passwordStored ? (
              <>
                Stored in your Keychain. Type to replace it
                {onRemovePassword && (
                  <>
                    {" "}or{" "}
                    <button type="button" className="text-accent hover:underline" onClick={onRemovePassword}>
                      remove it
                    </button>
                  </>
                )}
                .
              </>
            ) : (
              "Empty: log in with your SSH key. A password is kept in your Keychain."
            )
          }
        >
          <Input
            type="password"
            autoComplete="off"
            value={value.password}
            onChange={(e) => set({ password: e.target.value })}
            placeholder={passwordStored ? "••••••••" : "optional"}
          />
        </Field>
      </div>
    </>
  );
}

const MANUAL = "";

export function RemoteFields({
  value,
  onChange,
  passwordStored,
  onRemovePassword,
}: {
  value: RemoteForm;
  onChange: (v: RemoteForm) => void;
  passwordStored?: boolean;
  onRemovePassword?: () => void;
}) {
  const qc = useQueryClient();
  const { data: hosts = [] } = useSshHosts();
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const set = (patch: Partial<RemoteForm>) => onChange({ ...value, ...patch });
  const saved = hosts.find((h) => h.id === value.hostId);
  const { data: savedPassword = false } = useSshPasswordSet(saved ?? null);

  function pick(id: string) {
    const h = hosts.find((h) => h.id === id);
    set(h ? { ...connectionForm(h), hostId: h.id } : { hostId: null });
  }

  async function saveAsHost() {
    const conn = parseConnection(value);
    if (typeof conn === "string") return setError(conn);
    setError(null);
    setSaving(true);
    try {
      const host = await api.saveSshHost({ id: "", name: conn.host, ...conn }, value.password || null);
      await qc.invalidateQueries({ queryKey: keys.sshHosts });
      set({ ...connectionForm(host), hostId: host.id });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <>
      {(hosts.length > 0 || value.hostId) && (
        <Field label="Connection">
          <Select className="h-8 text-[13px]" value={value.hostId ?? MANUAL} onChange={(e) => pick(e.target.value)}>
            {hosts.map((h) => (
              <option key={h.id} value={h.id}>
                {h.name === h.host ? remoteLabel(h) : `${h.name} — ${remoteLabel(h)}`}
              </option>
            ))}
            {value.hostId && !saved && <option value={value.hostId}>Saved host (deleted)</option>}
            <option value={MANUAL}>Enter manually…</option>
          </Select>
        </Field>
      )}
      {saved ? (
        <p className="text-xs text-muted">
          Connects as <span className="font-mono text-fg">{remoteLabel(saved)}</span>
          {savedPassword ? " with a stored password" : " with your SSH key"}. Change it under Settings › SSH hosts; every
          project using this host follows.
        </p>
      ) : (
        <>
          <ConnectionFields
            value={value}
            onChange={(c) => set(c)}
            passwordStored={passwordStored}
            onRemovePassword={onRemovePassword}
          />
          <button
            type="button"
            className="self-start text-xs text-accent hover:underline disabled:opacity-40"
            disabled={saving || !value.host.trim()}
            onClick={saveAsHost}
          >
            {saving ? "Saving…" : "Save as a host to reuse it in other projects"}
          </button>
          <ErrorText>{error}</ErrorText>
        </>
      )}
      <Checkbox checked={value.keepAlive} onChange={(keepAlive) => set({ keepAlive })}>
        <span className="text-[13px]">Keep the remote session alive</span>
        <span className="block text-xs text-faint">
          Sessions run in tmux on the host and keep working when worki quits or the connection drops. worki installs
          tmux if it's missing.
        </span>
      </Checkbox>
    </>
  );
}
