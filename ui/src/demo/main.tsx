import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const scene = new URLSearchParams(location.search).get("scene") ?? "main";
const ROOT = "/Users/dev/worki/worktrees";
const now = Math.floor(Date.now() / 1000);

const projects = [
  project("acme-web", "/Users/dev/src/acme-web"),
  project("payments-api", "/Users/dev/src/payments-api"),
  project("infra", "/Users/dev/src/infra"),
];

function project(name: string, repoPath: string) {
  return {
    id: name,
    name,
    repoPath,
    worktreeRoot: `${ROOT}/${name}`,
    defaultBase: "main",
    isolation: { kind: "inherit" },
    setupScript: null,
    remote: null,
  };
}

const wt = (p: string, branch: string | null, isMain = false) => ({
  path: isMain ? projects.find((x) => x.id === p)!.repoPath : `${ROOT}/${p}/${branch!.replace(/\//g, "-")}`,
  branch: branch ?? "main",
  head: "abc1234",
  isMain,
  locked: false,
  prunable: false,
});

const worktrees: Record<string, ReturnType<typeof wt>[]> = {
  "acme-web": [wt("acme-web", null, true), wt("acme-web", "feat/checkout-redesign"), wt("acme-web", "fix/login-race"), wt("acme-web", "chore/react-19")],
  "payments-api": [wt("payments-api", null, true), wt("payments-api", "feat/refund-webhooks")],
  infra: [wt("infra", null, true), wt("infra", "feat/staging-cluster")],
};

const checkout = worktrees["acme-web"][1].path;
const login = worktrees["acme-web"][2].path;
const refunds = worktrees["payments-api"][1].path;
const staging = worktrees.infra[1].path;

const file = (path: string, code: string) => ({ path, code });
const status: Record<string, unknown> = {
  [checkout]: {
    changedFiles: [file("src/checkout/Summary.tsx", " M"), file("src/checkout/summary.css", "??"), file("src/checkout/Summary.test.tsx", " M")],
    insertions: 61,
    deletions: 9,
    ahead: 2,
    behind: 0,
    upstream: "origin/feat/checkout-redesign",
  },
  [login]: { changedFiles: [file("src/auth/session.ts", " M")], insertions: 12, deletions: 4, ahead: 0, behind: 0, upstream: null },
};
const clean = { changedFiles: [], insertions: 0, deletions: 0, ahead: 0, behind: 0, upstream: "origin/main" };

const commits = [
  { sha: "a1b2c3d4", subject: "Show a skeleton while the cart loads", author: "Claude", time: now - 25 * 60 },
  { sha: "b2c3d4e5", subject: "Render the checkout summary as a card", author: "Claude", time: now - 70 * 60 },
  { sha: "c3d4e5f6", subject: "Split shipping and discount into their own rows", author: "Claude", time: now - 3 * 3600 },
];

const isolation = { kind: "dockerSandbox", image: "claude", template: "worki-acme-web:20261109-1430" };

const session = (id: string, path: string, activity: string, extra: object = {}) => ({
  id,
  kind: "terminal",
  worktreePath: path,
  profileId: "claude",
  title: "Claude Code",
  runAs: "sbx: worki-" + path.split("/").pop(),
  resumable: false,
  state: { kind: "running" },
  activity,
  attention: null,
  createdAt: now,
  usage: [{ model: "claude-opus-5-5", input: 2400, output: 38_000, cacheWrite: 120_000, cacheWrite1h: 0, cacheRead: 1_400_000 }],
  agent: {
    model: "Opus 5.5",
    effort: "high",
    fastMode: false,
    thinking: true,
    permissionMode: "acceptEdits",
    contextUsed: 34,
    contextSize: 200_000,
    costUsd: 1.84,
    outputStyle: "default",
    version: "2.1.319",
  },
  ...extra,
});

const sessions = [
  session("s1", checkout, "working"),
  session("s2", login, "waiting", { attention: "Claude needs your permission to use Bash" }),
  session("s3", refunds, "working"),
  session("s4", staging, "waiting", { attention: "Which region should the staging cluster use?" }),
  session("s5", checkout, "waiting", {
    kind: "chat",
    attention: "Allow Bash?",
    createdAt: now + 1,
    agent: { model: "Opus 5.5", effort: "high", permissionMode: "default", contextUsed: 34, contextSize: 200_000, costUsd: 1.84, version: "2.1.319" },
  }),
];

const chatEvents: object[] = [
  {
    type: "options",
    permissionModes: ["default", "acceptEdits", "plan", "auto"],
    models: [
      { value: "default", model: "claude-opus-5-5", label: "Default (recommended)", description: "Opus 5.5", effortLevels: ["low", "medium", "high", "xhigh", "max"] },
      { value: "opus", model: "claude-opus-5-5", label: "Opus 5.5", description: "For complex work", effortLevels: ["low", "medium", "high", "xhigh", "max"] },
      { value: "sonnet", model: "claude-sonnet-5-5", label: "Sonnet 5.5", description: "Fast everyday coding", effortLevels: ["low", "medium", "high", "xhigh", "max"] },
      { value: "haiku", model: "claude-haiku-5-5", label: "Haiku 5.5", description: "Fastest", effortLevels: ["low", "medium", "high"] },
    ],
  },
  { type: "session", model: "claude-opus-5-5", permissionMode: "default", effort: "high" },
  {
    type: "user",
    text: "The empty cart shows a total of $0.00 with shipping. Hide the summary when the cart is empty and add a test.",
    attachments: ["empty-cart.png"],
  },
  { type: "text", id: "m1:0", parent: null, text: "I'll look at how the summary handles an empty cart first." },
  { type: "toolCall", id: "t1", name: "Read", parent: null, input: { file_path: `${checkout}/src/checkout/Summary.tsx` } },
  { type: "toolResult", id: "t1", parent: null, isError: false, output: "export function Summary({ cart }: Props) {\n  const subtotal = …" },
  {
    type: "toolCall",
    id: "t2",
    name: "Edit",
    parent: null,
    input: {
      file_path: `${checkout}/src/checkout/Summary.tsx`,
      old_string: "export function Summary({ cart }: Props) {",
      new_string: "export function Summary({ cart }: Props) {\n  if (cart.items.length === 0) return null;",
    },
  },
  { type: "toolResult", id: "t2", parent: null, isError: false, output: "The file was updated." },
  {
    type: "text",
    id: "m2:0",
    parent: null,
    text: "The summary now renders nothing for an **empty cart**. I added a test:\n\n- `hides the summary for an empty cart`\n\nNext I'll run the checkout tests.",
  },
  {
    type: "text",
    id: "m2b:0",
    parent: null,
    text: "How the summary decides what to show:\n\n```mermaid\nflowchart LR\n  A[Cart] --> B{Items?}\n  B -- none --> C[Hide summary]\n  B -- some --> D[Subtotal + shipping]\n  D --> E[Total]\n```\n\nAnd the card layout:\n\n```svg\n<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"240\" height=\"90\" viewBox=\"0 0 240 90\"><rect x=\"4\" y=\"4\" width=\"232\" height=\"82\" rx=\"8\" fill=\"#f6f8fa\" stroke=\"#d0d7de\"/><text x=\"16\" y=\"30\" font-size=\"13\" fill=\"#57606a\">Subtotal</text><text x=\"16\" y=\"52\" font-size=\"13\" fill=\"#57606a\">Shipping</text><text x=\"16\" y=\"74\" font-size=\"14\" font-weight=\"bold\" fill=\"#24292f\">Total</text><script>alert(1)</script></svg>\n```",
  },
  {
    type: "text",
    id: "m2c:0",
    parent: null,
    text: "Test timing per file:\n\n```html\n<div class=\"chart\"><h4>Checkout test time (ms)</h4><div class=\"row\"><span>Summary</span><i style=\"--w:82%\"></i><b>431</b></div><div class=\"row\"><span>LineItem</span><i style=\"--w:36%\"></i><b>188</b></div><div class=\"row\"><span>Cart</span><i style=\"--w:58%\"></i><b>302</b></div></div><style>.chart{padding:14px 16px;border:1px solid var(--line);border-radius:10px;background:var(--elevated)}h4{margin:0 0 10px;font-weight:600;color:var(--fg)}.row{display:grid;grid-template-columns:72px 1fr 40px;align-items:center;gap:10px;margin:6px 0;color:var(--muted)}i{display:block;height:10px;width:var(--w);border-radius:5px;background:var(--accent)}b{font-weight:500;color:var(--fg);text-align:right}</style>\n```",
  },
  {
    type: "permission",
    requestId: "p1",
    tool: "Bash",
    input: { command: "pnpm vitest run src/checkout", description: "Run the checkout tests" },
    questions: [],
  },
];
type DemoChannel = { onmessage?: (data: number[]) => void };
let chatSettings: Record<string, string> = { model: "claude-opus-5-5", permissionMode: "default", effort: "high" };
let chatChannel: DemoChannel | null = null;
const sendTo = (channel: DemoChannel | null, events: object[]) =>
  channel?.onmessage?.(Array.from(new TextEncoder().encode(events.map((e) => JSON.stringify(e) + "\n").join(""))));
const sendChat = (...events: object[]) => sendTo(chatChannel, events);

const esc = "\x1b[";
const c = (code: string, text: string) => `${esc}${code}m${text}${esc}0m`;
const terminal = [
  `${c("38;5;209", "✻")} ${c("1", "Welcome to Claude Code")}`,
  `  ${c("90", checkout)}`,
  "",
  `${c("1", ">")} Redesign the checkout summary as a card with subtotal, shipping and`,
  `  total rows, show a skeleton while loading, and keep the tests green.`,
  "",
  `${c("38;5;209", "●")} I'll start with the current summary and its tests.`,
  "",
  `${c("32", "●")} ${c("1", "Read")}(src/checkout/Summary.tsx)`,
  `  ${c("90", "└ Read 24 lines")}`,
  `${c("32", "●")} ${c("1", "Update")}(src/checkout/Summary.tsx)`,
  `  ${c("90", "└ Updated src/checkout/Summary.tsx with 21 additions and 7 removals")}`,
  `     ${c("90", " 8")} ${c("31", "-  const total = cart.items.reduce((sum, i) => sum + i.price * i.qty, 0);")}`,
  `     ${c("90", " 8")} ${c("32", "+  const subtotal = cart.items.reduce((sum, i) => sum + i.price * i.qty, 0);")}`,
  `     ${c("90", " 9")} ${c("32", "+  const total = subtotal + cart.shipping - cart.discount;")}`,
  `     ${c("90", "11")} ${c("32", "+  if (loading) return <LineItemSkeleton rows={3} />;")}`,
  "",
  `${c("32", "●")} ${c("1", "Bash")}(pnpm vitest run src/checkout)`,
  `  ${c("90", "└")} ${c("32", "✓")} src/checkout/Summary.test.tsx ${c("90", "(9 tests) 412ms")}`,
  `    ${c("32", "✓")} src/checkout/LineItem.test.tsx ${c("90", "(5 tests) 188ms")}`,
  `    ${c("1", "Test Files")}  ${c("32", "2 passed")} ${c("90", "(2)")}`,
  "",
  `${c("38;5;209", "✻ Polishing the empty-cart state…")} ${c("90", "(38s · ↓ 2.1k tokens · esc to interrupt)")}`,
  "",
].join("\r\n");

const diff = `diff --git a/src/checkout/Summary.tsx b/src/checkout/Summary.tsx
--- a/src/checkout/Summary.tsx
+++ b/src/checkout/Summary.tsx
@@ -1,20 +1,26 @@
 import { useCart } from "../cart";
 import { Money } from "../ui/Money";
+import { LineItemSkeleton } from "./LineItem";
+import "./summary.css";
 
 export function Summary() {
-  const { cart } = useCart();
-  const total = cart.items.reduce((sum, i) => sum + i.price * i.qty, 0);
+  const { cart, loading } = useCart();
+  const subtotal = cart.items.reduce((sum, i) => sum + i.price * i.qty, 0);
+  const total = subtotal + cart.shipping - cart.discount;
+
+  if (loading) return <LineItemSkeleton rows={3} />;
 
   return (
-    <div className="summary">
-      <Money value={total} />
-    </div>
+    <section className="summary-card">
+      <Row label="Subtotal" value={subtotal} />
+      <Row label="Shipping" value={cart.shipping} />
+      <Row label="Total" value={total} strong />
+    </section>
   );
 }
`;

const apps = [
  { id: "vscode", name: "Visual Studio Code", path: "/Applications/Visual Studio Code.app" },
  { id: "idea", name: "IntelliJ IDEA", path: "/Applications/IntelliJ IDEA.app" },
];

const check = (id: string, label: string) => ({ id, label, ok: true, detail: null, fix: null });

function handle(cmd: string, args: Record<string, any>): unknown {
  switch (cmd) {
    case "list_projects":
      return projects;
    case "list_worktrees":
      return worktrees[args.projectId] ?? [];
    case "worktree_status":
      return status[args.path] ?? clean;
    case "list_sessions":
      return sessions;
    case "agent_settings":
      return {
        profiles: [
          { id: "claude", name: "Claude Code", command: "claude", install: null },
          { id: "codex", name: "Codex", command: "codex", install: null },
        ],
        defaultProfile: "claude",
        secrets: {},
      };
    case "isolation_settings":
      return { default: isolation, stopOnQuit: false };
    case "worktree_isolation":
      return { choice: { kind: "inherit" }, effective: isolation };
    case "opener_settings":
      return { apps, defaultApp: "vscode" };
    case "app_icon":
      return null;
    case "compare_base":
      return `origin/${args.base}`;
    case "branch_changes":
      return {
        base: args.base,
        files: [file("src/checkout/Summary.tsx", "M "), file("src/checkout/LineItem.tsx", "M "), file("src/checkout/summary.css", "A "), file("src/checkout/Summary.test.tsx", "M ")],
        insertions: 142,
        deletions: 37,
        commits: 3,
        behind: 2,
      };
    case "branch_log":
      return args.worktreePath === checkout ? commits : args.worktreePath === login ? commits.slice(-1) : [];
    case "snapshots":
      return [
        { engine: "dockerSandbox", name: "worki-acme-web:20261104-1012", created: 1793783520, usedBy: ["acme-web"] },
        { engine: "dockerSandbox", name: "worki-acme-web:20261020-0931", created: 1792488660, usedBy: [] },
        { engine: "docker", name: "worki-payments-api:20261101-1630", created: 1793547000, usedBy: ["payments-api › feat-refund-webhooks"] },
        { engine: "docker", name: "worki-infra:20260930-0815", created: 1790748900, usedBy: [] },
      ];
    case "delete_snapshot":
      return null;
    case "draft_commits": {
      const files = (args as { files: string[] }).files;
      const split = files.length > 1;
      return new Promise((resolve) =>
        setTimeout(
          () =>
            resolve(
              split
                ? [
                    { message: "Serialize session refreshes behind the login", files: files.slice(0, -1) },
                    { message: "Fix a typo in the checkout summary", files: files.slice(-1) },
                  ]
                : [{ message: "Serialize session refreshes behind the login", files }],
            ),
          900,
        ),
      );
    }
    case "draft_pull_request":
      return new Promise((resolve) =>
        setTimeout(
          () =>
            resolve({
              title: "Fix the race between login and session refresh",
              body: "## Summary\nLogging in while a session refresh was in flight could drop the new token.\n\n- Serialize refreshes behind the login\n- Add a regression test",
            }),
          1200,
        ),
      );
    case "pull_request":
      return args.worktreePath === checkout
        ? { number: 42, title: "Redesign the checkout summary", url: "https://github.com/acme/web/pull/42", state: "OPEN", isDraft: false, checksPassed: 3, checksFailed: 0, checksPending: 1 }
        : null;
    case "file_diff":
      return diff;
    case "environment_templates":
      return ["worki-acme-web:20261109-1430"];
    case "isolation_doctor":
      return [
        check("binary", "Docker Sandboxes (sbx) is installed"),
        check("daemon", "The sandbox daemon is running"),
        check("image", "Template worki-acme-web:20261109-1430 is available"),
      ];
    case "agent_statuses":
      return [
        { profileId: "claude", name: "Claude Code", executable: "claude", envPath: "/usr/local/bin/claude", hostPath: "/opt/homebrew/bin/claude", canInstall: true, canShare: false, hasConfig: true, auth: { secret: "CLAUDE_CODE_OAUTH_TOKEN", set: true } },
        { profileId: "codex", name: "Codex", executable: "codex", envPath: null, hostPath: "/opt/homebrew/bin/codex", canInstall: true, canShare: false, hasConfig: false, auth: null },
      ];
    case "secret_status":
      return (args.names as string[]).map((n) => [n, true]);
    case "ssh_hosts":
      return [];
    case "setup_scripts":
      return { project: null, repoFile: "pnpm install" };
    case "attach_session": {
      const channel = args.onData;
      if (args.id === "s1") setTimeout(() => channel.onmessage?.(Array.from(new TextEncoder().encode(terminal))), 50);
      if (args.id === "s5") {
        chatChannel = channel;
        setTimeout(() => sendTo(channel, chatEvents), 50);
      }
      return 1;
    }
    case "send_chat_message": {
      sendChat({ type: "user", text: args.text, attachments: (args.attachments ?? []).map((a: { path?: string; name?: string }) => a.name ?? a.path?.split("/").pop()) });
      let i = 0;
      const words = "Sure — I'll take care of that next.".split(" ");
      const tick = () => {
        if (i < words.length) {
          sendChat({ type: "textDelta", id: `r${now}`, parent: null, text: (i ? " " : "") + words[i++] });
          setTimeout(tick, 120);
        } else sendChat({ type: "turnEnded", ok: true, interrupted: false, error: null });
      };
      setTimeout(tick, 300);
      return null;
    }
    case "answer_chat_permission": {
      const allowed = args.answer.kind !== "deny";
      sendChat({ type: "permissionResolved", requestId: args.requestId, allowed });
      if (allowed)
        sendChat(
          { type: "toolCall", id: "t3", name: "Bash", parent: null, input: { command: "pnpm vitest run src/checkout", description: "Run the checkout tests" } },
          { type: "toolResult", id: "t3", parent: null, isError: false, output: " ✓ src/checkout/Summary.test.tsx (10 tests) 431ms\n Test Files  1 passed (1)" },
          { type: "text", id: "m3:0", parent: null, text: "All 10 checkout tests pass." },
          { type: "turnEnded", ok: true, interrupted: false, error: null },
        );
      return null;
    }
    case "change_chat_setting": {
      const resolved: Record<string, string> = { default: "claude-opus-5-5", opus: "claude-opus-5-5", sonnet: "claude-sonnet-5-5", haiku: "claude-haiku-5-5" };
      chatSettings = {
        ...chatSettings,
        ...(args.setting.kind === "model" ? { model: resolved[args.setting.value] } : { [args.setting.kind]: args.setting.value }),
      };
      sendChat({ type: "session", ...chatSettings });
      return null;
    }
    case "session_continue_command":
      return `sbx exec -it -w ${checkout} -e IS_SANDBOX=1 worki-feat-checkout-redesign-3f2a bash -lc 'claude --resume ${args.id}'`;
    case "interrupt_chat":
      sendChat({ type: "notice", text: "Request interrupted by user" }, { type: "turnEnded", ok: false, interrupted: true, error: null });
      return null;
    case "restore_agent_sessions":
      return { started: 0, failed: [] };
    case "restore_remote_sessions":
      return 0;
    default:
      return null;
  }
}

mockWindows("main");
mockIPC((cmd, args) => handle(cmd, (args ?? {}) as Record<string, any>), { shouldMockEvents: true });

const ui = {
  selectedProjectId: "acme-web",
  selectedWorktreePath: checkout,
  collapsed: {},
  showChanges: true,
  changesScope: scene === "branch" ? "branch" : "uncommitted",
  panelWidths: { sidebar: 256, diff: 520, changes: 320 },
  theme: "dark",
  terminalFontSize: 13,
  reopenSessions: true,
  activeSession: scene === "chat" ? { [checkout]: "s5" } : {},
  showCommits: true,
  showPullRequests: true,
};
localStorage.setItem("worki-ui", JSON.stringify({ state: ui, version: 0 }));
localStorage.removeItem("worki-review");

await import("../main");

const { useUi } = await import("../lib/store");
if (scene === "settings") useUi.getState().openDialog({ kind: "settings", tab: "isolation" });
if (scene === "git") useUi.getState().openDialog({ kind: "settings", tab: "git" });
if (scene === "isolation") {
  const open = () => {
    const badge = [...document.querySelectorAll("button")].find((b) => b.title.includes("click to change"));
    if (badge) badge.click();
    else setTimeout(open, 100);
  };
  open();
}
