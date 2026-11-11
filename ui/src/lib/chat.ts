import { useEffect, useState } from "react";

import { api, type BackgroundTask, type ChatEvent, type ChatQuestion, type ModelChoice } from "./api";

export type ChatItem =
  | {
      kind: "user";
      key: string;
      text: string;
      attachments: string[];
      /** Set for messages sent in this session. */
      messageId: string | null;
      /** Sent while the agent was busy and not read by it yet. */
      queued: boolean;
    }
  | { kind: "text"; key: string; text: string; done: boolean }
  | {
      kind: "tool";
      key: string;
      name: string;
      input: Record<string, unknown>;
      result: { output: string; isError: boolean } | null;
      /** What a subagent did for this tool call. */
      children: ChatItem[];
    }
  | {
      kind: "permission";
      key: string;
      tool: string;
      input: Record<string, unknown>;
      questions: ChatQuestion[];
      state: "open" | "allowed" | "denied";
    }
  | { kind: "notice"; key: string; text: string; tone: "info" | "error" };

export interface ChatSettings {
  model: string | null;
  permissionMode: string | null;
  effort: string | null;
}

export interface Chat {
  items: ChatItem[];
  settings: ChatSettings | null;
  options: { models: ModelChoice[]; permissionModes: string[] } | null;
  exited: { code: number; error: string | null } | null;
  /** Open permission requests, oldest first. */
  open: Extract<ChatItem, { kind: "permission" }>[];
  /** What the agent does between tool calls, such as `thinking`; null once the turn ends. */
  status: { status: string; tool: string | null } | null;
  /** When the current turn started, in ms. */
  turnStarted: number | null;
  background: (BackgroundTask & { started: number })[];
}

/** Builds the conversation from chat events. A changed item is replaced by a new object, so
 * memoized rows only re-render when their own item changed. */
export class ChatModel {
  private items: ChatItem[] = [];
  private byKey = new Map<string, ChatItem>();
  /** The tool call an item belongs to, for items a subagent added. */
  private parentOf = new Map<string, string>();
  private exited: Chat["exited"] = null;
  private settings: Chat["settings"] = null;
  private options: Chat["options"] = null;
  private status: Chat["status"] = null;
  private turnStarted: number | null = null;
  private background: Chat["background"] = [];
  /** Item keys of the messages the agent hasn't read yet, by message id. */
  private unread = new Map<string, string>();
  private seq = 0;

  apply(event: ChatEvent) {
    switch (event.type) {
      case "user": {
        const key = `u${this.seq++}`;
        const messageId = event.id ?? null;
        const queued = messageId !== null && this.turnStarted !== null;
        if (messageId) this.unread.set(messageId, key);
        if (!queued) this.turnStarted = Date.now();
        this.push(null, {
          kind: "user",
          key,
          text: event.text,
          attachments: event.attachments ?? [],
          messageId,
          queued,
        });
        break;
      }
      case "userRead":
        this.turnStarted ??= Date.now();
        this.markRead(this.unread.get(event.id));
        this.unread.delete(event.id);
        break;
      case "textDelta": {
        const item = this.byKey.get(event.id);
        if (item?.kind === "text") this.replace({ ...item, text: item.text + event.text });
        else this.push(event.parent, { kind: "text", key: event.id, text: event.text, done: false });
        break;
      }
      case "text": {
        const item = this.byKey.get(event.id);
        if (item?.kind === "text") this.replace({ ...item, text: event.text, done: true });
        else this.push(event.parent, { kind: "text", key: event.id, text: event.text, done: true });
        break;
      }
      case "toolCall":
        if (!this.byKey.has(event.id))
          this.push(event.parent, {
            kind: "tool",
            key: event.id,
            name: event.name,
            input: asObject(event.input),
            result: null,
            children: [],
          });
        break;
      case "toolResult": {
        const item = this.byKey.get(event.id);
        if (item?.kind === "tool") this.replace({ ...item, result: { output: event.output, isError: event.isError } });
        break;
      }
      case "permission":
        this.push(null, {
          kind: "permission",
          key: event.requestId,
          tool: event.tool,
          input: asObject(event.input),
          questions: event.questions,
          state: "open",
        });
        break;
      case "permissionResolved": {
        const item = this.byKey.get(event.requestId);
        if (item?.kind === "permission") this.replace({ ...item, state: event.allowed ? "allowed" : "denied" });
        break;
      }
      case "notice":
        this.push(null, { kind: "notice", key: `n${this.seq++}`, text: event.text, tone: "info" });
        break;
      case "status":
        this.status = { status: event.status, tool: event.tool };
        break;
      case "backgroundTasks": {
        const seen = new Map(this.background.map((t) => [t.id, t.started]));
        this.background = event.tasks.map((t) => ({ ...t, started: seen.get(t.id) ?? Date.now() }));
        break;
      }
      case "turnEnded":
        this.status = null;
        this.turnStarted = null;
        this.closeOpen();
        if (event.error)
          this.push(null, { kind: "notice", key: `n${this.seq++}`, text: event.error, tone: "error" });
        break;
      case "exited":
        this.status = null;
        this.turnStarted = null;
        this.background = [];
        for (const key of this.unread.values()) this.markRead(key);
        this.unread.clear();
        this.closeOpen();
        this.exited = { code: event.code, error: event.error };
        break;
      case "session":
        this.settings = { model: event.model, permissionMode: event.permissionMode, effort: event.effort };
        break;
      case "options":
        this.options = { models: event.models, permissionModes: event.permissionModes };
        break;
    }
  }

  snapshot(): Chat {
    const open = this.items.filter(
      (i): i is Extract<ChatItem, { kind: "permission" }> => i.kind === "permission" && i.state === "open",
    );
    return {
      items: [...this.items],
      settings: this.settings,
      options: this.options,
      exited: this.exited,
      open,
      status: this.status,
      turnStarted: this.turnStarted,
      background: this.background,
    };
  }

  private push(parent: string | null, item: ChatItem) {
    const owner = parent ? this.byKey.get(parent) : undefined;
    this.byKey.set(item.key, item);
    if (owner?.kind === "tool") {
      this.parentOf.set(item.key, owner.key);
      this.replace({ ...owner, children: [...owner.children, item] });
    } else this.items.push(item);
  }

  /** Swaps in a changed item, and copies the tool calls it sits in, so React sees the change. */
  private replace(item: ChatItem) {
    this.byKey.set(item.key, item);
    const parentKey = this.parentOf.get(item.key);
    const parent = parentKey ? this.byKey.get(parentKey) : undefined;
    if (parent?.kind === "tool") {
      this.replace({ ...parent, children: parent.children.map((c) => (c.key === item.key ? item : c)) });
      return;
    }
    const at = this.items.findIndex((i) => i.key === item.key);
    if (at >= 0) this.items[at] = item;
  }

  private markRead(key: string | undefined) {
    const item = key ? this.byKey.get(key) : undefined;
    if (item?.kind === "user" && item.queued) this.replace({ ...item, queued: false });
  }

  private closeOpen() {
    for (const item of [...this.byKey.values()])
      if (item.kind === "permission" && item.state === "open") this.replace({ ...item, state: "denied" });
  }
}

function asObject(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};
}

/** Attaches to a chat session and keeps its conversation up to date, at most once a frame. */
export function useChat(sessionId: string): Chat | null {
  const [chat, setChat] = useState<Chat | null>(null);
  useEffect(() => {
    const model = new ChatModel();
    const decoder = new TextDecoder();
    let partial = "";
    let frame = 0;
    let disposed = false;
    let sinkId: number | null = null;
    const render = () => {
      frame = 0;
      if (!disposed) setChat(model.snapshot());
    };
    api
      .attachSession(sessionId, (bytes) => {
        const text = partial + decoder.decode(bytes, { stream: true });
        const lines = text.split("\n");
        partial = lines.pop() ?? "";
        for (const line of lines) {
          if (!line.trim()) continue;
          try {
            model.apply(JSON.parse(line) as ChatEvent);
          } catch {
            // A line that isn't an event is skipped; the rest of the conversation still shows.
          }
        }
        if (!frame) frame = requestAnimationFrame(render);
      })
      .then((id) => {
        sinkId = id;
        if (disposed) api.detachSession(sessionId, id);
        else render();
      });
    return () => {
      disposed = true;
      if (frame) cancelAnimationFrame(frame);
      if (sinkId !== null) api.detachSession(sessionId, sinkId);
    };
  }, [sessionId]);
  return chat;
}
