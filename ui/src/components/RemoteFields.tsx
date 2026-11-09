import type { Remote } from "../lib/api";
import { Checkbox, Field, Input } from "./ui";

interface RemoteForm {
  host: string;
  user: string;
  port: string;
  password: string;
  keepAlive: boolean;
}

export function remoteForm(r: Remote | null): RemoteForm {
  return { host: r?.host ?? "", user: r?.user ?? "", port: r?.port ? String(r.port) : "", password: "", keepAlive: r?.keepAlive ?? true };
}

/** The connection settings, or an error message when they're not valid. */
export function parseRemote(f: RemoteForm): Remote | string {
  const host = f.host.trim();
  if (!host) return "Enter a host.";
  const port = f.port.trim() ? Number(f.port.trim()) : null;
  if (port !== null && (!Number.isInteger(port) || port < 1 || port > 65535)) return "The port must be a number from 1 to 65535.";
  return { host, user: f.user.trim() || null, port, keepAlive: f.keepAlive };
}

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
  const set = (patch: Partial<RemoteForm>) => onChange({ ...value, ...patch });
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
