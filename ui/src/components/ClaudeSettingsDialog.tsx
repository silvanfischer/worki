import { useQueryClient } from "@tanstack/react-query";
import { Download, LoaderCircle, RotateCcw } from "lucide-react";
import { useState } from "react";

import { api, errorMessage, type JsonObject } from "../lib/api";
import { keys } from "../lib/queries";
import { Button, ErrorText, NestedDialog, Textarea } from "./ui";

const isObject = (v: unknown): v is JsonObject => typeof v === "object" && v !== null && !Array.isArray(v);

/** `over` wins where both set a value; objects are merged key by key. */
function merge(base: JsonObject, over: JsonObject): JsonObject {
  const out: JsonObject = { ...base };
  for (const [key, value] of Object.entries(over)) {
    out[key] = isObject(out[key]) && isObject(value) ? merge(out[key], value) : value;
  }
  return out;
}

const format = (settings: JsonObject) => JSON.stringify(settings, null, 2);

function parse(text: string): { value: JsonObject } | { error: string } {
  if (!text.trim()) return { value: {} };
  try {
    const value: unknown = JSON.parse(text);
    return isObject(value) ? { value } : { error: "The settings must be a JSON object: { … }" };
  } catch (e) {
    return { error: errorMessage(e) };
  }
}

export function ClaudeSettingsDialog({ settings, onClose }: { settings: JsonObject; onClose: () => void }) {
  const qc = useQueryClient();
  const [text, setText] = useState(() => format(settings));
  const [busy, setBusy] = useState<"import" | "save" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const parsed = parse(text);
  const valid = "value" in parsed;

  async function attempt(action: () => Promise<void>) {
    try {
      await action();
      setError(null);
    } catch (e) {
      setError(errorMessage(e));
    }
  }

  const reset = () => attempt(async () => setText(format(await api.defaultClaudeSettings())));

  async function importHost() {
    if (!valid) return;
    setBusy("import");
    await attempt(async () => setText(format(merge(await api.hostClaudeSettings(), parsed.value))));
    setBusy(null);
  }

  async function save() {
    if (!valid) return;
    setBusy("save");
    await attempt(async () => {
      await api.setClaudeSettings(parsed.value);
      await qc.invalidateQueries({ queryKey: keys.agentSettings });
      onClose();
    });
    setBusy(null);
  }

  return (
    <NestedDialog
      title="Claude Code settings for isolated sessions"
      onClose={onClose}
      width={640}
      footer={
        <>
          <Button
            variant="ghost"
            onClick={reset}
            title="Puts worki's default settings in the editor. Nothing changes until you save."
            className="mr-auto"
          >
            <RotateCcw size={12} />
            Reset to default
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" onClick={save} disabled={busy !== null || !valid}>
            {busy === "save" && <LoaderCircle size={12} className="animate-spin" />}
            Save
          </Button>
        </>
      }
    >
      <p className="text-xs leading-relaxed text-muted">
        Passed as written with <span className="font-mono">--settings</span> to sessions in User Isolation, Docker
        Sandboxes, Docker and Podman, not over SSH or without isolation. In terminals, worki also adds its status{" "}
        <span className="font-mono">hooks</span> and <span className="font-mono">statusLine</span>, and{" "}
        <span className="font-mono">remoteControlAtStartup</span> when Remote Control is on.
      </p>
      <Textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={18}
        className="min-h-64 resize-y leading-relaxed"
        aria-label="Settings JSON"
      />
      <ErrorText>{valid ? error : parsed.error}</ErrorText>
      <div className="flex items-center gap-3">
        <Button onClick={importHost} disabled={busy !== null || !valid} className="shrink-0">
          {busy === "import" ? <LoaderCircle size={12} className="animate-spin" /> : <Download size={12} />}
          Import my settings
        </Button>
        <span className="text-xs leading-relaxed text-faint">
          Merges in <span className="font-mono">~/.claude/settings.json</span>, without hooks, status line, sandbox and
          deny or ask rules.
        </span>
      </div>
    </NestedDialog>
  );
}
