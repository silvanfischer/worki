import clsx from "clsx";
import { ChevronDown, ChevronsUpDown, ChevronUp, MessageSquarePlus, Send, Trash2, X } from "lucide-react";
import { Fragment, useMemo, useState } from "react";

import { type DiffLine, diffRows, type GapReveal, parseDiff } from "../lib/diff";
import { errorMessage } from "../lib/api";
import { useFileDiff, useSessions } from "../lib/queries";
import {
  type ReviewComment,
  reviewPrompt,
  reviewTarget,
  sendReview,
  useReview,
  useWorktreeComments,
} from "../lib/review";
import { useUi } from "../lib/store";
import { Button, DiffStat, ErrorText, IconButton, Textarea } from "./ui";

/** Keeps comment rows in view while the code beside them scrolls sideways. */
const PINNED = "sticky left-0 w-[100cqw] px-3";

const REVEAL_STEP = 20;

type CodeLine = Extract<DiffLine, { kind: "context" | "add" | "del" }>;

function anchor(l: CodeLine): { side: "new" | "old"; line: number } {
  return l.kind === "del" ? { side: "old", line: l.oldNo! } : { side: "new", line: l.newNo! };
}

export function DiffView({
  worktreePath,
  file,
  base,
  onClose,
}: {
  worktreePath: string;
  file: string;
  base: string | null;
  onClose: () => void;
}) {
  const { data, error, isLoading } = useFileDiff(worktreePath, file, base);
  const comments = useWorktreeComments(worktreePath).filter((c) => c.file === file);
  const addComment = useReview((s) => s.add);
  const [drafting, setDrafting] = useState<string | null>(null);
  const { data: sessions = [] } = useSessions();
  const activeId = useUi((s) => s.activeSession[worktreePath]);
  const setActive = useUi((s) => s.setActiveSession);
  const target = reviewTarget(
    sessions.filter((s) => s.worktreePath === worktreePath),
    activeId,
  );
  const parsed = useMemo(() => (data !== undefined ? parseDiff(data) : null), [data]);
  const [revealed, setRevealed] = useState<Record<number, GapReveal>>({});
  const rows = useMemo(() => {
    if (!parsed) return [];
    const commented = new Set(comments.map((c) => `${c.side}:${c.line}`));
    const keep = new Set<number>();
    parsed.lines.forEach((l, i) => {
      if (l.kind !== "context") return;
      if (commented.has(`new:${l.newNo}`) || drafting === `new:${l.newNo}`) keep.add(i);
    });
    return diffRows(parsed.lines, revealed, keep);
  }, [parsed, revealed, comments, drafting]);
  const reveal = (id: number, patch: Partial<GapReveal>) =>
    setRevealed((r) => {
      const cur = r[id] ?? { head: 0, tail: 0 };
      return { ...r, [id]: { head: cur.head + (patch.head ?? 0), tail: cur.tail + (patch.tail ?? 0) } };
    });
  const added = parsed?.lines.filter((l) => l.kind === "add").length ?? 0;
  const removed = parsed?.lines.filter((l) => l.kind === "del").length ?? 0;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex h-13 shrink-0 items-center gap-2 border-b border-line px-3">
        <span className="selectable min-w-0 flex-1 truncate pl-1 font-mono fs-diff">{file}</span>
        {base && <span className="fs-diff-sm text-faint">vs {base}</span>}
        {parsed && <DiffStat insertions={added} deletions={removed} />}
        <IconButton label="Close diff" onClick={onClose}>
          <X size={14} />
        </IconButton>
      </div>
      <div className="selectable @container min-h-0 flex-1 overflow-auto font-mono fs-diff leading-[1.6]">
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
              {rows.map((row) => {
                if (row.kind === "gap")
                  return (
                    <GapRow
                      key={`gap:${row.id}`}
                      hidden={row.end - row.start}
                      atStart={row.start === 0}
                      atEnd={row.end === parsed.lines.length}
                      onReveal={(patch) => reveal(row.id, patch)}
                    />
                  );
                const i = row.index;
                const l = parsed.lines[i];
                if (l.kind === "hunk") return null;
                if (l.kind === "note")
                  return (
                    <tr key={i} className="text-faint">
                      <td colSpan={2} />
                      <td className="px-3 italic">{l.text}</td>
                    </tr>
                  );
                const at = anchor(l);
                const key = `${at.side}:${at.line}`;
                const here = comments.filter((c) => c.side === at.side && c.line === at.line);
                return (
                  <Fragment key={i}>
                    <tr
                      className={clsx(
                        "group",
                        l.kind === "add" && "bg-added/12",
                        l.kind === "del" && "bg-removed/12",
                      )}
                    >
                      <td className="w-[4.5em] border-r border-line px-2 text-right text-faint select-none">
                        {l.oldNo ?? ""}
                      </td>
                      <td className="relative w-[4.5em] border-r border-line px-2 text-right text-faint select-none">
                        <button
                          title="Comment on this line"
                          onClick={() => setDrafting(key)}
                          className="absolute top-0 -left-1 hidden size-[19px] items-center justify-center rounded bg-accent text-accent-fg group-hover:flex"
                        >
                          <MessageSquarePlus size={11} />
                        </button>
                        {l.newNo ?? ""}
                      </td>
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
                    {here.map((c) => (
                      <CommentRow key={c.id} worktreePath={worktreePath} comment={c} />
                    ))}
                    {drafting === key && (
                      <tr>
                        <td colSpan={3} className="border-y border-line bg-panel py-2 font-sans">
                          <div className={PINNED}>
                            <CommentEditor
                              initial=""
                              submitLabel="Add comment"
                              sendTo={target?.title}
                              onSend={async (text) => {
                                await sendReview(target!, reviewPrompt([{ id: "", file, ...at, code: l.text, text }]));
                                setActive(worktreePath, target!.id);
                                setDrafting(null);
                              }}
                              onCancel={() => setDrafting(null)}
                              onSubmit={(text) => {
                                addComment(worktreePath, { file, ...at, code: l.text, text });
                                setDrafting(null);
                              }}
                            />
                          </div>
                        </td>
                      </tr>
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        )}
        {parsed && <OrphanComments worktreePath={worktreePath} lines={parsed.lines} comments={comments} />}
      </div>
    </div>
  );
}

function GapRow({
  hidden,
  atStart,
  atEnd,
  onReveal,
}: {
  hidden: number;
  atStart: boolean;
  atEnd: boolean;
  onReveal: (patch: Partial<GapReveal>) => void;
}) {
  const step = Math.min(REVEAL_STEP, hidden);
  const button = "flex items-center gap-1 rounded px-1.5 hover:bg-accent/20 hover:text-fg";
  return (
    <tr className="bg-accent/10 text-muted select-none">
      <td colSpan={3} className="py-0.5 font-sans fs-diff-sm">
        <div className={clsx(PINNED, "flex items-center gap-2")}>
          {!atStart && (
            <button className={button} title={`Show ${step} more lines below`} onClick={() => onReveal({ head: step })}>
              <ChevronDown size={12} />
              {step}
            </button>
          )}
          {!atEnd && (
            <button className={button} title={`Show ${step} more lines above`} onClick={() => onReveal({ tail: step })}>
              <ChevronUp size={12} />
              {step}
            </button>
          )}
          {hidden > step && (
            <button className={button} onClick={() => onReveal({ head: hidden })}>
              <ChevronsUpDown size={12} />
              Show all
            </button>
          )}
          <span className="text-faint">
            {hidden} unchanged {hidden === 1 ? "line" : "lines"}
          </span>
        </div>
      </td>
    </tr>
  );
}

function CommentRow({ worktreePath, comment }: { worktreePath: string; comment: ReviewComment }) {
  const update = useReview((s) => s.update);
  const remove = useReview((s) => s.remove);
  const [editing, setEditing] = useState(false);
  return (
    <tr>
      <td colSpan={3} className="border-y border-line bg-panel py-2 font-sans">
        <div className={PINNED}>
          {editing ? (
            <CommentEditor
              initial={comment.text}
              submitLabel="Save"
              onCancel={() => setEditing(false)}
              onSubmit={(text) => {
                update(worktreePath, comment.id, text);
                setEditing(false);
              }}
            />
          ) : (
            <div className="flex items-start gap-2">
              <p className="flex-1 cursor-text fs-diff whitespace-pre-wrap" onClick={() => setEditing(true)}>
                {comment.text}
              </p>
              <IconButton label="Delete comment" onClick={() => remove(worktreePath, comment.id)}>
                <Trash2 size={12} />
              </IconButton>
            </div>
          )}
        </div>
      </td>
    </tr>
  );
}

/** Comments whose line is no longer in the diff, e.g. after the agent changed the file. */
function OrphanComments({
  worktreePath,
  lines,
  comments,
}: {
  worktreePath: string;
  lines: DiffLine[];
  comments: ReviewComment[];
}) {
  const shown = new Set(
    lines.flatMap((l) => (l.kind === "context" || l.kind === "add" || l.kind === "del" ? [anchor(l)] : [])).map(
      (a) => `${a.side}:${a.line}`,
    ),
  );
  const orphans = comments.filter((c) => !shown.has(`${c.side}:${c.line}`));
  if (orphans.length === 0) return null;
  return (
    <div className="border-t border-line p-3 font-sans">
      <p className="mb-2 fs-diff-sm font-semibold tracking-wide text-faint uppercase">Comments on lines not in the diff</p>
      <table className="w-full">
        <tbody>
          {orphans.map((c) => (
            <Fragment key={c.id}>
              <tr>
                <td className="font-mono fs-diff-sm text-faint">
                  {c.side === "old" ? "removed line " : "line "}
                  {c.line}: {c.code.trim()}
                </td>
              </tr>
              <CommentRow worktreePath={worktreePath} comment={c} />
            </Fragment>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function CommentEditor({
  initial,
  submitLabel,
  sendTo,
  onSubmit,
  onSend,
  onCancel,
}: {
  initial: string;
  submitLabel: string;
  sendTo?: string;
  onSubmit: (text: string) => void;
  onSend?: (text: string) => Promise<void>;
  onCancel: () => void;
}) {
  const [text, setText] = useState(initial);
  const [error, setError] = useState<string | null>(null);
  const submit = () => text.trim() && onSubmit(text.trim());
  const canSend = !!onSend && !!sendTo;
  const send = () => {
    if (!text.trim() || !canSend) return;
    setError(null);
    onSend(text.trim()).catch((e) => setError(errorMessage(e)));
  };
  return (
    <div className="flex flex-col gap-2">
      <Textarea
        autoFocus
        rows={3}
        value={text}
        onChange={(e) => setText(e.target.value)}
        placeholder="What should the agent change here?"
        className="font-sans fs-diff"
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
            e.preventDefault();
            if (e.metaKey || e.ctrlKey || !canSend) submit();
            else send();
          }
          if (e.key === "Escape") onCancel();
        }}
      />
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant={canSend ? undefined : "primary"}
          onClick={submit}
          disabled={!text.trim()}
          title={canSend ? "⌘↩" : "↩"}
        >
          {submitLabel}
        </Button>
        {onSend && (
          <Button
            variant={canSend ? "primary" : undefined}
            onClick={send}
            disabled={!text.trim() || !canSend}
            title={canSend ? `Send only this comment to ${sendTo} now (↩)` : "Start an agent in this worktree to send comments"}
          >
            <Send size={12} />
            Send to {sendTo ?? "agent"}
          </Button>
        )}
      </div>
      <ErrorText>{error}</ErrorText>
    </div>
  );
}
