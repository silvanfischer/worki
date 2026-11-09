import { useQueryClient } from "@tanstack/react-query";
import clsx from "clsx";
import { CircleCheck, CircleX, LoaderCircle, RefreshCw } from "lucide-react";
import { useState } from "react";

import { api, type Check, errorMessage, type Fix, type Isolation, type LogLine } from "../lib/api";
import { keys, useDoctor } from "../lib/queries";
import { Button, ErrorText, IconButton, LogView } from "./ui";

const fixLabel: Record<Fix, string> = {
  provision: "Set up…",
  prepare: "Configure",
  grantAccess: "Apply access rules",
  buildImage: "Build image",
};

export function DoctorPanel({ isolation, projectId }: { isolation: Isolation; projectId: string | null }) {
  const qc = useQueryClient();
  const { data: checks, isFetching, error, refetch } = useDoctor(isolation, projectId);
  const [running, setRunning] = useState<string | null>(null);
  const [fixError, setFixError] = useState<string | null>(null);
  const [log, setLog] = useState<{ checkId: string; lines: LogLine[] } | null>(null);

  async function fix(check: Check) {
    const f = check.fix!;
    setRunning(check.id);
    setFixError(null);
    try {
      if (f === "buildImage") {
        setLog({ checkId: check.id, lines: [] });
        await api.buildIsolationImage(isolation, (line) =>
          setLog((l) => (l ? { ...l, lines: [...l.lines.slice(-400), line] } : l)),
        );
      } else {
        await api.isolationFix(isolation, f, projectId);
      }
    } catch (e) {
      setFixError(errorMessage(e));
    } finally {
      setRunning(null);
      qc.invalidateQueries({ queryKey: keys.allDoctor });
    }
  }

  if (isolation.kind === "none") return null;

  return (
    <div className="flex flex-col gap-1 rounded-lg border border-line bg-panel p-2">
      <div className="flex items-center justify-between px-1 pb-1">
        <span className="text-[11px] font-semibold tracking-wide text-faint uppercase">Checks</span>
        <IconButton label="Re-run checks" onClick={() => refetch()} disabled={isFetching}>
          <RefreshCw size={12} className={clsx(isFetching && "animate-spin")} />
        </IconButton>
      </div>
      {!checks && isFetching && <p className="px-1 py-1 text-xs text-faint">Running checks…</p>}
      {error && <ErrorText>{String(error)}</ErrorText>}
      {checks?.map((c) => (
        <div key={c.id}>
          <CheckRow check={c} running={running} onFix={fix} />
          {log?.checkId === c.id && log.lines.length > 0 && (
            <LogView lines={log.lines} className="mx-1 mb-1 ml-7 max-h-40 bg-elevated" />
          )}
        </div>
      ))}
      {isolation.kind === "macUser" && checks?.some((c) => c.fix === "provision") && (
        <p className="px-1 pt-1 text-xs leading-relaxed text-faint">
          Setup creates a hidden user and a sudoers rule that only lets you run commands as worki agent users. macOS
          asks for an administrator password once.
        </p>
      )}
      <ErrorText>{fixError}</ErrorText>
    </div>
  );
}

function CheckRow({
  check,
  running,
  onFix,
}: {
  check: Check;
  running: string | null;
  onFix: (c: Check) => void;
}) {
  return (
    <div className="flex items-start gap-2 rounded-md px-1 py-1">
      {check.ok ? (
        <CircleCheck size={14} className="mt-0.5 shrink-0 text-added" />
      ) : (
        <CircleX size={14} className="mt-0.5 shrink-0 text-removed" />
      )}
      <div className="min-w-0 flex-1">
        <div className="text-[13px]">{check.label}</div>
        {check.detail && <div className="selectable text-xs break-words text-faint">{check.detail}</div>}
      </div>
      {check.fix && (
        <Button className="shrink-0" onClick={() => onFix(check)} disabled={running !== null}>
          {running === check.id && <LoaderCircle size={12} className="animate-spin" />}
          {fixLabel[check.fix]}
        </Button>
      )}
    </div>
  );
}
