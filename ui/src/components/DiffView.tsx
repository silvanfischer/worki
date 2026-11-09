import clsx from "clsx";
import { X } from "lucide-react";

import { parseDiff } from "../lib/diff";
import { useFileDiff } from "../lib/queries";
import { DiffStat, ErrorText, IconButton } from "./ui";

export function DiffView({ worktreePath, file, onClose }: { worktreePath: string; file: string; onClose: () => void }) {
  const { data, error, isLoading } = useFileDiff(worktreePath, file);
  const parsed = data !== undefined ? parseDiff(data) : null;
  const added = parsed?.lines.filter((l) => l.kind === "add").length ?? 0;
  const removed = parsed?.lines.filter((l) => l.kind === "del").length ?? 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line px-2">
        <span className="selectable min-w-0 flex-1 truncate pl-1 font-mono text-xs">{file}</span>
        {parsed && <DiffStat insertions={added} deletions={removed} />}
        <IconButton label="Close diff" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </div>
      <div className="selectable min-h-0 flex-1 overflow-auto font-mono text-xs leading-[1.6]">
        {isLoading && <p className="p-3 text-faint">Loading diff…</p>}
        {error && (
          <div className="p-3">
            <ErrorText>{String(error)}</ErrorText>
          </div>
        )}
        {parsed?.binary && <p className="p-3 text-faint">Binary file changed.</p>}
        {parsed && !parsed.binary && parsed.lines.length === 0 && (
          <p className="p-3 text-faint">No changes in this file anymore.</p>
        )}
        {parsed && parsed.lines.length > 0 && (
          <table className="w-full border-collapse">
            <tbody>
              {parsed.lines.map((l, i) =>
                l.kind === "hunk" ? (
                  <tr key={i} className="bg-accent/10 text-muted">
                    <td colSpan={3} className="px-3 py-0.5">
                      {l.text}
                    </td>
                  </tr>
                ) : l.kind === "note" ? (
                  <tr key={i} className="text-faint">
                    <td colSpan={2} />
                    <td className="px-3 italic">{l.text}</td>
                  </tr>
                ) : (
                  <tr
                    key={i}
                    className={clsx(l.kind === "add" && "bg-added/12", l.kind === "del" && "bg-removed/12")}
                  >
                    <td className="w-12 border-r border-line px-2 text-right text-faint select-none">{l.oldNo ?? ""}</td>
                    <td className="w-12 border-r border-line px-2 text-right text-faint select-none">{l.newNo ?? ""}</td>
                    <td className="px-3 whitespace-pre">
                      <span
                        className={clsx(
                          "mr-2 select-none",
                          l.kind === "add" ? "text-added" : l.kind === "del" ? "text-removed" : "text-faint",
                        )}
                      >
                        {l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}
                      </span>
                      {l.text}
                    </td>
                  </tr>
                ),
              )}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
