import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import clsx from "clsx";
import {
  ArrowUp,
  Check,
  ChevronRight,
  CircleAlert,
  LoaderCircle,
  MessageCircleQuestion,
  Paperclip,
  ShieldQuestion,
  Square,
  X,
} from "lucide-react";
import { isValidElement, memo, type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

import {
  api,
  type ChatAttachment,
  type ChatQuestion,
  type ChatSetting,
  errorMessage,
  type ModelChoice,
  type PermissionAnswer,
  type SessionInfo,
} from "../lib/api";
import { type Chat, type ChatItem, useChat } from "../lib/chat";
import { PERMISSION_MODES } from "./AgentStatus";
import { ChatVisual, VISUAL_LANGUAGES } from "./ChatVisual";
import { Button, ErrorText, Select, Textarea } from "./ui";

type Permission = Extract<ChatItem, { kind: "permission" }>;
type Tool = Extract<ChatItem, { kind: "tool" }>;

/** An agent session shown as a conversation instead of a terminal. */
export function ChatView({ session }: { session: SessionInfo }) {
  const chat = useChat(session.id);
  const scroller = useRef<HTMLDivElement>(null);
  const pinned = useRef(true);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && pinned.current) el.scrollTop = el.scrollHeight;
  });

  if (!chat)
    return (
      <div className="flex h-full items-center justify-center text-muted">
        <LoaderCircle size={16} className="animate-spin" />
      </div>
    );

  const running = session.state.kind === "running";
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div
        ref={scroller}
        onScroll={(e) => {
          const el = e.currentTarget;
          pinned.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <div className="mx-auto flex max-w-5xl flex-col gap-3 px-5 py-4">
          {chat.items.length === 0 && (
            <p className="py-10 text-center fs-chat-xs text-faint">Ask {session.title} something to start.</p>
          )}
          {chat.items.map((item) => (
            <Item key={item.key} item={item} session={session} />
          ))}
          {session.activity === "working" && chat.open.length === 0 && (
            <div className="flex items-center gap-2 fs-chat-xs text-muted">
              <LoaderCircle size={12} className="animate-spin" />
              Working…
            </div>
          )}
          {chat.exited?.error && <ErrorText>{chat.exited.error}</ErrorText>}
        </div>
      </div>
      {running && <Composer session={session} chat={chat} onSend={() => (pinned.current = true)} />}
    </div>
  );
}

const Item = memo(function Item({ item, session }: { item: ChatItem; session: SessionInfo }) {
  switch (item.kind) {
    case "user":
      return (
        <div className="flex max-w-[85%] flex-col gap-1.5 self-end rounded-lg bg-selected px-3 py-2 fs-chat whitespace-pre-wrap">
          {item.text}
          {item.attachments.length > 0 && (
            <div className="flex flex-wrap gap-1">
              {item.attachments.map((name, i) => (
                <span key={i} className="flex items-center gap-1 rounded bg-hover px-1.5 py-0.5 fs-chat-xs text-muted">
                  <Paperclip size={10} />
                  {name}
                </span>
              ))}
            </div>
          )}
        </div>
      );
    case "text":
      return <MarkdownText text={item.text} />;
    case "tool":
      return <ToolCall tool={item} session={session} />;
    case "permission":
      return item.state === "open" ? (
        <PermissionCard request={item} session={session} />
      ) : (
        <div className="flex items-center gap-1.5 fs-chat-xs text-faint">
          {item.state === "allowed" ? <Check size={11} /> : <X size={11} />}
          {item.questions.length > 0
            ? item.state === "allowed"
              ? "Answered"
              : "Not answered"
            : `${item.state === "allowed" ? "Allowed" : "Denied"} ${item.tool}`}
        </div>
      );
    case "notice":
      return (
        <div
          className={clsx(
            "flex items-center gap-1.5 fs-chat-xs",
            item.tone === "error" ? "text-removed" : "text-faint",
          )}
        >
          {item.tone === "error" && <CircleAlert size={11} />}
          {item.text}
        </div>
      );
  }
});

const markdown: Components = {
  a: ({ href, children }) => (
    <a
      href={href}
      onClick={(e) => {
        e.preventDefault();
        if (href) openUrl(href).catch(() => {});
      }}
      className="text-accent underline-offset-2 hover:underline"
    >
      {children}
    </a>
  ),
  p: ({ children }) => <p className="my-1.5 leading-relaxed">{children}</p>,
  ul: ({ children }) => <ul className="my-1.5 list-disc pl-5">{children}</ul>,
  ol: ({ children }) => <ol className="my-1.5 list-decimal pl-5">{children}</ol>,
  li: ({ children }) => <li className="my-0.5">{children}</li>,
  h1: ({ children }) => <h3 className="mt-3 mb-1 fs-chat-xl font-semibold">{children}</h3>,
  h2: ({ children }) => <h3 className="mt-3 mb-1 fs-chat-lg font-semibold">{children}</h3>,
  h3: ({ children }) => <h4 className="mt-2 mb-1 font-semibold">{children}</h4>,
  blockquote: ({ children }) => <blockquote className="my-1.5 border-l-2 border-line pl-3 text-muted">{children}</blockquote>,
  pre: ({ children }) => {
    const code = isValidElement<{ className?: string; children?: ReactNode }>(children) ? children.props : null;
    const language = /language-(\w+)/.exec(code?.className ?? "")?.[1];
    if (language && VISUAL_LANGUAGES.includes(language))
      return <ChatVisual language={language} source={String(code?.children ?? "").replace(/\n$/, "")} />;
    return (
      <pre className="my-2 overflow-x-auto rounded-md bg-hover px-3 py-2 font-mono fs-chat-xs [&_code]:bg-transparent [&_code]:p-0">
        {children}
      </pre>
    );
  },
  code: ({ children }) => <code className="rounded bg-hover px-1 py-0.5 font-mono fs-chat-sm">{children}</code>,
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="fs-chat-xs [&_td]:border [&_td]:border-line [&_td]:px-2 [&_td]:py-1 [&_th]:border [&_th]:border-line [&_th]:px-2 [&_th]:py-1">
        {children}
      </table>
    </div>
  ),
};

function MarkdownText({ text }: { text: string }) {
  return (
    <div className="fs-chat break-words">
      <Markdown remarkPlugins={[remarkGfm]} components={markdown}>
        {text}
      </Markdown>
    </div>
  );
}

function relative(path: unknown, root: string): string {
  const p = typeof path === "string" ? path : "";
  return p.startsWith(`${root}/`) ? p.slice(root.length + 1) : p;
}

/** One line saying what the tool call does. */
function summary(name: string, input: Record<string, unknown>, root: string): string {
  const str = (key: string) => (typeof input[key] === "string" ? (input[key] as string) : "");
  switch (name) {
    case "Bash":
      return str("description") || str("command");
    case "Read":
    case "Write":
    case "Edit":
    case "MultiEdit":
    case "NotebookEdit":
      return relative(input.file_path ?? input.notebook_path, root);
    case "Grep":
    case "Glob":
      return str("pattern");
    case "WebFetch":
      return str("url");
    case "WebSearch":
      return str("query");
    case "Task":
    case "Agent":
      return str("description");
    case "TodoWrite":
      return "Updated the to-do list";
    default: {
      const first = Object.values(input).find((v) => typeof v === "string");
      return typeof first === "string" ? first : "";
    }
  }
}

function ToolCall({ tool, session }: { tool: Tool; session: SessionInfo }) {
  const root = session.worktreePath;
  const [open, setOpen] = useState(false);
  const failed = tool.result?.isError;
  return (
    <div className="flex flex-col">
      <button
        onClick={() => setOpen((o) => !o)}
        className="flex min-w-0 items-center gap-1.5 rounded px-1 py-0.5 text-left fs-chat-xs text-muted hover:bg-hover hover:text-fg"
      >
        <ChevronRight size={12} className={clsx("shrink-0 transition-transform", open && "rotate-90")} />
        <span className={clsx("shrink-0 font-medium", failed ? "text-removed" : "text-fg")}>{tool.name}</span>
        <span className="min-w-0 truncate font-mono fs-chat-xs">{summary(tool.name, tool.input, root)}</span>
        {!tool.result && <LoaderCircle size={11} className="shrink-0 animate-spin" />}
      </button>
      {open && (
        <div className="ml-4 flex flex-col gap-2 border-l border-line py-1 pl-3">
          <ToolInput name={tool.name} input={tool.input} />
          {tool.result && <Output text={tool.result.output} error={tool.result.isError} />}
          {tool.children.map((child) => (
            <Item key={child.key} item={child} session={session} />
          ))}
        </div>
      )}
    </div>
  );
}

/** What the tool was asked to do: a diff for edits, the command for shells, else the input. */
function ToolInput({ name, input }: { name: string; input: Record<string, unknown> }) {
  if (name === "Edit") return <Changes edits={[input]} />;
  if (name === "MultiEdit" && Array.isArray(input.edits)) return <Changes edits={input.edits as Record<string, unknown>[]} />;
  if (name === "Write" && typeof input.content === "string")
    return <Changes edits={[{ old_string: "", new_string: input.content }]} />;
  if (name === "Bash" && typeof input.command === "string")
    return <pre className="overflow-x-auto font-mono fs-chat-xs whitespace-pre-wrap">$ {input.command}</pre>;
  if (name === "ExitPlanMode" && typeof input.plan === "string") return <MarkdownText text={input.plan} />;
  return <Output text={JSON.stringify(input, null, 2)} />;
}

function Changes({ edits }: { edits: Record<string, unknown>[] }) {
  const lines = edits.flatMap((e) => [
    ...String(e.old_string ?? "")
      .split("\n")
      .filter((l, i, all) => l || i < all.length - 1)
      .map((text) => ({ kind: "del" as const, text })),
    ...String(e.new_string ?? "")
      .split("\n")
      .filter((l, i, all) => l || i < all.length - 1)
      .map((text) => ({ kind: "add" as const, text })),
  ]);
  return <Lines lines={lines} />;
}

const COLLAPSED = 12;

function Lines({ lines }: { lines: { kind: "add" | "del" | "plain"; text: string }[] }) {
  const [all, setAll] = useState(false);
  const shown = all ? lines : lines.slice(0, COLLAPSED);
  return (
    <div className="overflow-x-auto rounded-md border border-line font-mono fs-chat-xs leading-[1.6]">
      {shown.map((l, i) => (
        <div
          key={i}
          className={clsx(
            "px-2 whitespace-pre",
            l.kind === "add" && "bg-added/10 text-added",
            l.kind === "del" && "bg-removed/10 text-removed",
          )}
        >
          {l.kind === "add" ? "+ " : l.kind === "del" ? "- " : ""}
          {l.text || " "}
        </div>
      ))}
      {lines.length > COLLAPSED && (
        <button onClick={() => setAll((a) => !a)} className="w-full px-2 py-0.5 text-left text-faint hover:bg-hover">
          {all ? "Show less" : `${lines.length - COLLAPSED} more lines`}
        </button>
      )}
    </div>
  );
}

function Output({ text, error }: { text: string; error?: boolean }) {
  if (!text.trim()) return null;
  const lines = text.replace(/\n$/, "").split("\n");
  return error ? (
    <div className="fs-chat-xs whitespace-pre-wrap text-removed">{text}</div>
  ) : (
    <Lines lines={lines.map((t) => ({ kind: "plain", text: t }))} />
  );
}

function useAnswer(session: SessionInfo, request: Permission) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const answer = async (a: PermissionAnswer) => {
    setBusy(true);
    setError(null);
    try {
      await api.answerChatPermission(session.id, request.key, a);
    } catch (e) {
      setError(errorMessage(e));
      setBusy(false);
    }
  };
  return { busy, error, answer };
}

function PermissionCard({ request, session }: { request: Permission; session: SessionInfo }) {
  if (request.questions.length > 0) return <QuestionCard request={request} session={session} />;
  return <AllowCard request={request} session={session} />;
}

function Card({ icon, title, children }: { icon: ReactNode; title: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-2.5 rounded-lg border border-accent/40 bg-elevated p-3 shadow-sm">
      <div className="flex items-center gap-2 fs-chat font-medium">
        <span className="text-accent">{icon}</span>
        {title}
      </div>
      {children}
    </div>
  );
}

function AllowCard({ request, session }: { request: Permission; session: SessionInfo }) {
  const { busy, error, answer } = useAnswer(session, request);
  const plan = request.tool === "ExitPlanMode";
  const allow = useRef<HTMLButtonElement>(null);
  useEffect(() => allow.current?.focus(), []);
  return (
    <Card
      icon={<ShieldQuestion size={14} />}
      title={plan ? "Ready to start on this plan?" : `Allow ${request.tool}?`}
    >
      {!plan && (
        <span className="truncate font-mono fs-chat-xs text-muted">
          {summary(request.tool, request.input, session.worktreePath)}
        </span>
      )}
      <ToolInput name={request.tool} input={request.input} />
      <div className="flex gap-2">
        <Button ref={allow} variant="primary" disabled={busy} onClick={() => answer({ kind: "allow" })}>
          {plan ? "Start" : "Allow"}
        </Button>
        <Button disabled={busy} onClick={() => answer({ kind: "deny", message: null })}>
          {plan ? "Keep planning" : "Deny"}
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
    </Card>
  );
}

function QuestionCard({ request, session }: { request: Permission; session: SessionInfo }) {
  const { busy, error, answer } = useAnswer(session, request);
  const [picked, setPicked] = useState<Record<string, string[]>>({});
  const [other, setOther] = useState<Record<string, string>>({});
  const value = (q: ChatQuestion) => [...(picked[q.question] ?? []), ...(other[q.question]?.trim() ? [other[q.question].trim()] : [])];
  const complete = request.questions.every((q) => value(q).length > 0);
  const submit = (answers = Object.fromEntries(request.questions.map((q) => [q.question, value(q).join(", ")]))) =>
    answer({ kind: "answers", answers });

  const choose = (q: ChatQuestion, label: string) => {
    if (!q.multiSelect && request.questions.length === 1) {
      submit({ [q.question]: label });
      return;
    }
    setPicked((p) => {
      const now = p[q.question] ?? [];
      const next = q.multiSelect ? (now.includes(label) ? now.filter((l) => l !== label) : [...now, label]) : [label];
      return { ...p, [q.question]: next };
    });
  };

  return (
    <Card icon={<MessageCircleQuestion size={14} />} title={request.questions.length === 1 ? request.questions[0].question : "Questions"}>
      {request.questions.map((q) => (
        <div key={q.question} className="flex flex-col gap-1.5">
          {request.questions.length > 1 && <span className="fs-chat-xs font-medium">{q.question}</span>}
          <div className="flex flex-col gap-1">
            {q.options.map((o) => {
              const on = picked[q.question]?.includes(o.label);
              return (
                <button
                  key={o.label}
                  disabled={busy}
                  onClick={() => choose(q, o.label)}
                  className={clsx(
                    "flex flex-col rounded-md border px-2.5 py-1.5 text-left hover:bg-hover",
                    on ? "border-accent bg-accent/10" : "border-line",
                  )}
                >
                  <span className="fs-chat">{o.label}</span>
                  {o.description && <span className="fs-chat-xs text-muted">{o.description}</span>}
                </button>
              );
            })}
            <input
              value={other[q.question] ?? ""}
              onChange={(e) => setOther((o) => ({ ...o, [q.question]: e.target.value }))}
              onKeyDown={(e) => e.key === "Enter" && complete && submit()}
              placeholder="Something else…"
              disabled={busy}
              className="h-8 rounded-md border border-line bg-panel px-2.5 fs-chat focus:border-accent focus:outline-none"
            />
          </div>
        </div>
      ))}
      <div className="flex gap-2">
        {(request.questions.length > 1 || request.questions.some((q) => q.multiSelect) || Object.values(other).some((o) => o.trim())) && (
          <Button variant="primary" disabled={busy || !complete} onClick={() => submit()}>
            Answer
          </Button>
        )}
        <Button variant="ghost" disabled={busy} onClick={() => answer({ kind: "deny", message: "The user skipped the question." })}>
          Skip
        </Button>
      </div>
      <ErrorText>{error}</ErrorText>
    </Card>
  );
}

interface Pending {
  id: number;
  name: string;
  attachment: ChatAttachment;
  /** Data URL of a pasted image, for its thumbnail. */
  preview: string | null;
}

let nextPending = 0;

/** The largest image Claude's API takes. */
const MAX_IMAGE = 5 * 1024 * 1024;

function readImage(file: File): Promise<Pending> {
  return new Promise((resolve, reject) => {
    if (file.size > MAX_IMAGE) {
      reject(new Error(`${file.name || "The pasted image"} is too large to attach (images can be up to 5 MB)`));
      return;
    }
    const reader = new FileReader();
    reader.onerror = () => reject(reader.error);
    reader.onload = () => {
      const url = String(reader.result);
      resolve({
        id: nextPending++,
        name: file.name || "Pasted image",
        attachment: { kind: "image", name: file.name || "pasted image", mediaType: file.type, data: url.slice(url.indexOf(",") + 1) },
        preview: url,
      });
    };
    reader.readAsDataURL(file);
  });
}

function Composer({ session, chat, onSend }: { session: SessionInfo; chat: Chat; onSend: () => void }) {
  const [text, setText] = useState("");
  const [attached, setAttached] = useState<Pending[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  useEffect(() => input.current?.focus(), [session.id]);
  const ready = !!text.trim() || attached.length > 0;
  const busy = session.activity !== "idle";

  async function send() {
    if (!ready || sending) return;
    setError(null);
    setSending(true);
    try {
      await api.sendChatMessage(session.id, text.trim(), attached.map((a) => a.attachment));
      setText("");
      setAttached([]);
      onSend();
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setSending(false);
    }
  }

  async function pick() {
    const picked = await openDialog({ multiple: true, title: "Attach files" }).catch(() => null);
    const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
    setAttached((a) => [
      ...a,
      ...paths.map((path) => ({
        id: nextPending++,
        name: path.split("/").pop() ?? path,
        attachment: { kind: "file" as const, path },
        preview: null,
      })),
    ]);
    input.current?.focus();
  }

  async function paste(e: React.ClipboardEvent) {
    const images = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"));
    if (images.length === 0) return;
    e.preventDefault();
    try {
      const read = await Promise.all(images.map(readImage));
      setAttached((a) => [...a, ...read]);
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  const stop = () => api.interruptChat(session.id).catch((e) => setError(errorMessage(e)));

  return (
    <div className="shrink-0 border-t border-line px-5 py-3">
      <div className="mx-auto flex max-w-5xl flex-col gap-1.5">
        <div className="flex flex-col gap-1.5 rounded-lg border border-line bg-panel px-2 py-1.5 focus-within:border-accent">
          {attached.length > 0 && (
            <div className="flex flex-wrap gap-1.5 px-1 pt-0.5">
              {attached.map((a) => (
                <span key={a.id} className="flex items-center gap-1.5 rounded-md border border-line bg-elevated py-0.5 pr-1 pl-1.5 fs-chat-xs">
                  {a.preview ? <img src={a.preview} alt="" className="size-5 rounded object-cover" /> : <Paperclip size={11} className="text-muted" />}
                  <span className="max-w-40 truncate">{a.name}</span>
                  <button
                    aria-label={`Remove ${a.name}`}
                    onClick={() => setAttached((all) => all.filter((x) => x.id !== a.id))}
                    className="flex size-4 items-center justify-center rounded text-muted hover:bg-hover hover:text-fg"
                  >
                    <X size={10} />
                  </button>
                </span>
              ))}
            </div>
          )}
        <div className="flex items-end gap-2">
          <button
            onClick={pick}
            title="Attach files (or paste an image)"
            className="flex size-7 shrink-0 items-center justify-center rounded-md text-muted hover:bg-hover hover:text-fg"
          >
            <Paperclip size={14} />
          </button>
          <Textarea
            ref={input}
            rows={1}
            maxRows={10}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onPaste={paste}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                send();
              } else if (e.key === "Escape" && busy) {
                e.preventDefault();
                stop();
              }
            }}
            placeholder={busy ? "Add to what the agent is doing…" : `Message ${session.title}`}
            className="flex-1 border-transparent! bg-transparent px-1 py-1 font-sans fs-chat outline-none!"
          />
          {busy && !ready ? (
            <button
              onClick={stop}
              title="Stop (Esc)"
              className="flex size-7 shrink-0 items-center justify-center rounded-md bg-fg text-panel hover:opacity-80"
            >
              <Square size={11} fill="currentColor" />
            </button>
          ) : (
            <button
              onClick={send}
              disabled={!ready || sending}
              title="Send (Enter)"
              className="flex size-7 shrink-0 items-center justify-center rounded-md bg-accent text-accent-fg hover:brightness-110 disabled:opacity-40"
            >
              <ArrowUp size={14} />
            </button>
          )}
        </div>
        </div>
        <Settings session={session} chat={chat} onError={setError} />
        <ErrorText>{error}</ErrorText>
      </div>
    </div>
  );
}

/** The model, permission mode and effort the agent runs with, switchable mid-conversation. */
function Settings({ session, chat, onError }: { session: SessionInfo; chat: Chat; onError: (e: string | null) => void }) {
  const [pickedModel, setPickedModel] = useState<string | null>(null);
  const { settings, options } = chat;
  if (!settings || !options) return null;

  const models = options.models;
  const fallback = models.find((m) => m.value === "default");
  // Before its first turn the agent hasn't said which model it runs; that's its default.
  const current = settings.model
    ? (models.find((m) => m.value === pickedModel && m.model === settings.model) ??
      models.find((m) => m.model === settings.model && m.value !== "default") ??
      models.find((m) => m.model === settings.model))
    : (models.find((m) => m.value === pickedModel) ?? fallback);
  const label = (m: ModelChoice) => {
    if (m.value !== "default") return m === fallback ? m.label : `${m.label}${m.model && m.model === fallback?.model ? " (default)" : ""}`;
    const named = models.find((o) => o.value !== "default" && o.model === m.model);
    return `Default (${named?.label ?? m.model ?? "recommended"})`;
  };
  // A level the selected model doesn't have (after switching models) falls back to its default.
  const effort = settings.effort && current?.effortLevels.includes(settings.effort) ? settings.effort : "";
  const change = (setting: ChatSetting) => {
    onError(null);
    api.changeChatSetting(session.id, setting).catch((e) => onError(errorMessage(e)));
  };
  const select = "h-6 max-w-44 border-transparent bg-transparent fs-chat-xs text-muted hover:bg-hover hover:text-fg";

  return (
    <div className="flex flex-wrap items-center gap-1">
      <Select
        title="Model"
        value={current?.value ?? ""}
        onChange={(e) => {
          setPickedModel(e.target.value);
          change({ kind: "model", value: e.target.value });
        }}
        className={select}
      >
        {!current && <option value="">{settings.model ?? "Model"}</option>}
        {models.map((m) => (
          <option key={m.value} value={m.value} title={m.description ?? undefined}>
            {label(m)}
          </option>
        ))}
      </Select>
      {current && current.effortLevels.length > 0 && (
        <Select
          title="Effort"
          value={effort}
          onChange={(e) => change({ kind: "effort", value: e.target.value })}
          className={select}
        >
          {!effort && <option value="">Default effort</option>}
          {current.effortLevels.map((level) => (
            <option key={level} value={level}>
              {level} effort
            </option>
          ))}
        </Select>
      )}
      <Select
        title="Permissions"
        value={settings.permissionMode ?? ""}
        onChange={(e) => change({ kind: "permissionMode", value: e.target.value })}
        className={select}
      >
        {settings.permissionMode && !options.permissionModes.includes(settings.permissionMode) && (
          <option value={settings.permissionMode}>
            {PERMISSION_MODES[settings.permissionMode] ?? settings.permissionMode}
          </option>
        )}
        {options.permissionModes.map((mode) => (
          <option key={mode} value={mode}>
            {PERMISSION_MODES[mode] ?? mode}
          </option>
        ))}
      </Select>
    </div>
  );
}
