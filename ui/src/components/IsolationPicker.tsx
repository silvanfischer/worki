import { DEFAULT_AGENT_USER, DEFAULT_CONTAINER_IMAGE, DEFAULT_SANDBOX_IMAGE, type ProjectIsolation, SANDBOX_IMAGES } from "../lib/api";
import { Input, Select } from "./ui";

type Kind = ProjectIsolation["kind"];

const options: { kind: Kind; label: string; hint: string }[] = [
  { kind: "inherit", label: "Global default", hint: "" },
  { kind: "none", label: "None", hint: "Agents run as you, with full access to your account." },
  {
    kind: "macUser",
    label: "User Isolation",
    hint: "Agents run as a separate hidden macOS user that can only reach the worktrees assigned to it.",
  },
  {
    kind: "dockerSandbox",
    label: "Docker Sandbox (sbx)",
    hint: "Each worktree gets its own Docker Sandbox, a separate Linux microVM. Needs Docker Sandboxes (sbx).",
  },
  {
    kind: "docker",
    label: "Docker",
    hint: "Each worktree gets its own Docker container. Needs Docker Desktop.",
  },
  {
    kind: "podman",
    label: "Podman",
    hint: "Each worktree gets its own container in the Podman machine VM. Needs Podman.",
  },
];

export function IsolationPicker<T extends ProjectIsolation>({
  value,
  onChange,
  allowInherit,
  inheritLabel,
  inheritName = "Global default",
}: {
  value: T;
  onChange: (v: T) => void;
  allowInherit?: boolean;
  inheritLabel?: string;
  /** What "inherit" means here: the global default (projects) or the project default (worktrees). */
  inheritName?: string;
}) {
  const visible = options.filter((o) => allowInherit || o.kind !== "inherit");
  const current = visible.find((o) => o.kind === value.kind);

  function pick(kind: Kind) {
    if (kind === value.kind) return;
    const next: ProjectIsolation =
      kind === "macUser"
        ? { kind, user: DEFAULT_AGENT_USER }
        : kind === "dockerSandbox"
          ? { kind, image: DEFAULT_SANDBOX_IMAGE }
          : kind === "docker" || kind === "podman"
            ? { kind, image: DEFAULT_CONTAINER_IMAGE }
            : ({ kind } as ProjectIsolation);
    onChange(next as T);
  }

  return (
    <div className="flex flex-col gap-2">
      <Select
        value={value.kind}
        onChange={(e) => pick(e.target.value as Kind)}
        className="h-8 w-full max-w-96 text-[13px]"
      >
        {visible.map((o) => (
          <option key={o.kind} value={o.kind}>
            {o.kind === "inherit" ? (inheritLabel ? `${inheritName} (${inheritLabel})` : inheritName) : o.label}
          </option>
        ))}
      </Select>
      {current?.hint && <p className="text-xs text-faint">{current.hint}</p>}
      {value.kind === "macUser" && (
        <label className="flex items-center gap-2">
          <span className="text-xs text-muted">User name</span>
          <Input
            className="max-w-56 font-mono text-xs"
            value={value.user}
            onChange={(e) => onChange({ kind: "macUser", user: e.target.value.trim() } as T)}
          />
        </label>
      )}
      {(value.kind === "docker" || value.kind === "podman") && (
        <label className="flex flex-col gap-1">
          <span className="flex items-center gap-2">
            <span className="text-xs text-muted">Image</span>
            <Input
              className="max-w-72 font-mono text-xs"
              value={value.image}
              onChange={(e) => onChange({ kind: value.kind, image: e.target.value.trim() } as T)}
            />
          </span>
          <span className="text-xs text-faint">
            {value.image === DEFAULT_CONTAINER_IMAGE
              ? "worki's agent image (Debian, git, gh, node, Claude Code), built by Set up below."
              : "Any image with bash and git."}
          </span>
        </label>
      )}
      {value.kind === "dockerSandbox" && (
        <label className="flex items-center gap-2">
          <span className="text-xs text-muted">Image</span>
          <Select
            value={value.image}
            onChange={(e) => onChange({ kind: "dockerSandbox", image: e.target.value } as T)}
            className="h-8 text-[13px]"
          >
            {SANDBOX_IMAGES.map((img) => (
              <option key={img} value={img}>
                {img}
              </option>
            ))}
          </Select>
        </label>
      )}
    </div>
  );
}
