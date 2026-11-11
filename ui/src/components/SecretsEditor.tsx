import { Plus, Trash2 } from "lucide-react";
import { useState } from "react";

import { api, errorMessage, type Isolation, isolationLabel, scopeKey } from "../lib/api";
import { useAgentSettings, useIsolationSettings, useProjects, useRefreshSecrets, useSecretStatus } from "../lib/queries";
import { Button, ErrorText, IconButton, Input, Select } from "./ui";

/** Secrets per isolation: agents only get the ones stored for where they run. Saved immediately. */
export function SecretsEditor() {
  const { data: settings } = useAgentSettings();
  const { data: isolationSettings } = useIsolationSettings();
  const { data: projects = [] } = useProjects();

  const scopes = new Map<string, string>([["none", "Agents running as me"]]);
  const add = (i: Isolation) => i.kind !== "none" && scopes.set(scopeKey(i), isolationLabel(i));
  if (isolationSettings) add(isolationSettings.default);
  for (const p of projects) {
    if (p.remote) add({ kind: "ssh", ...p.remote });
    else if (p.isolation.kind === "macUser") add(p.isolation);
  }
  for (const key of Object.keys(settings?.secrets ?? {})) if (!scopes.has(key)) scopes.set(key, key);

  const [scope, setScope] = useState(() =>
    isolationSettings ? scopeKey(isolationSettings.default) : "none",
  );
  const names = settings?.secrets[scope] ?? [];
  const { data: status = [] } = useSecretStatus(scope, names);
  const stored = new Set(status.filter(([, ok]) => ok).map(([n]) => n));

  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);

  const refresh = useRefreshSecrets();

  async function save() {
    setError(null);
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) return setError("Use an environment variable name, e.g. GH_TOKEN.");
    if (!value) return setError("Enter a value.");
    try {
      await api.setSecret(scope, name, value);
      setName("");
      setValue("");
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  async function remove(n: string) {
    try {
      await api.deleteSecret(scope, n);
      await refresh();
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  return (
    <div className="flex flex-col gap-2">
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-medium text-muted">Secrets</span>
        <Select value={scope} onChange={(e) => setScope(e.target.value)} className="h-7 text-xs">
          {[...scopes].map(([key, label]) => (
            <option key={key} value={key}>
              {label}
            </option>
          ))}
        </Select>
      </div>
      <p className="text-xs leading-relaxed text-faint">
        Stored in your Keychain and passed as environment variables only to agents running here. Saved right away.
      </p>
      {names.map((n) => (
        <div key={n} className="flex items-center gap-2 px-1">
          <span className="flex-1 font-mono text-xs">{n}</span>
          <span className="text-[11px] text-faint">{stored.has(n) ? "stored" : "no value"}</span>
          <IconButton label={`Remove ${n}`} onClick={() => remove(n)}>
            <Trash2 size={13} />
          </IconButton>
        </div>
      ))}
      <div className="grid grid-cols-[200px_1fr_auto] items-center gap-2 px-1">
        <Input
          value={name}
          onChange={(e) => setName(e.target.value.trim())}
          placeholder="GH_TOKEN"
          className="font-mono text-xs"
        />
        <Input type="password" value={value} onChange={(e) => setValue(e.target.value)} placeholder="Value" />
        <Button onClick={save} disabled={!name || !value}>
          <Plus size={13} />
          Add
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
    </div>
  );
}
