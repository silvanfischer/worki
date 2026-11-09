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
  ...extra,
});

const sessions = [
  session("s1", checkout, "working"),
  session("s2", login, "waiting", { attention: "Claude needs your permission to use Bash" }),
  session("s3", refunds, "working"),
  session("s4", staging, "waiting", { attention: "Which region should the staging cluster use?" }),
];

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
      return args.worktreePath === checkout ? commits : [];
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
      return 1;
    }
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
  gitTaskProfile: null,
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
