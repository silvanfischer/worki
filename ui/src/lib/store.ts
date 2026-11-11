import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { SessionKind } from "./api";
import type { AskPrompt } from "./askPrompts";

export type DialogState =
  | { kind: "newWorktree"; projectId: string }
  | { kind: "removeWorktree"; projectId: string; path: string }
  | { kind: "saveEnvironment"; projectId: string; path: string }
  | { kind: "projectSettings"; projectId: string }
  | { kind: "addRemoteProject" }
  | { kind: "settings"; tab: SettingsTab }
  | null;

export type SettingsTab = "general" | "appearance" | "agents" | "git" | "secrets" | "isolation" | "snapshots" | "sshHosts" | "openIn";

export interface PrDraft {
  title: string;
  body: string;
}

export type ChangesScope = "uncommitted" | "branch";

export type Theme = "system" | "light" | "dark";

export const FONT_SIZES = [11, 12, 13, 14, 15, 16, 18];

export type Panel = "sidebar" | "diff" | "changes";

export const PANEL_LIMITS: Record<Panel, { min: number; max: number; initial: number }> = {
  sidebar: { min: 180, max: 480, initial: 256 },
  diff: { min: 280, max: 1400, initial: 560 },
  changes: { min: 180, max: 600, initial: 280 },
};

interface UiState {
  selectedProjectId: string | null;
  selectedWorktreePath: string | null;
  collapsed: Record<string, boolean>;
  /** Last focused session per worktree path. */
  activeSession: Record<string, string>;
  /** Pull request title and description an agent wrote, per worktree path, until the PR is created. */
  prDrafts: Record<string, PrDraft>;
  /** Changed "Ask agent" prompts; missing ones use the defaults in askPrompts.ts. */
  askPrompts: Partial<Record<AskPrompt, string>>;
  /** Command that writes commit messages and pull requests in the background; empty uses
   * `DEFAULT_WRITER_COMMAND`. */
  writerCommand: string;
  showChanges: boolean;
  /** Whether the changes panel lists uncommitted changes or everything since the base branch. */
  changesScope: ChangesScope;
  panelWidths: Record<Panel, number>;
  theme: Theme;
  terminalFontSize: number;
  panelFontSize: number;
  diffFontSize: number;
  chatFontSize: number;
  sidebarFontSize: number;
  /** Start the Claude sessions that were open at the last quit again when worki starts. */
  reopenSessions: boolean;
  /** How agents that can chat open by default: as a chat or in a terminal. */
  agentView: SessionKind;
  /** Look for new worki releases on start and every few hours. */
  autoUpdate: boolean;
  /** Commit box, commit list and rebasing in the changes panel. */
  showCommits: boolean;
  /** Pull request section and status, which ask GitHub through `gh`. */
  showPullRequests: boolean;
  dialog: DialogState;
  select: (projectId: string, worktreePath: string | null) => void;
  toggleCollapsed: (projectId: string) => void;
  setActiveSession: (worktreePath: string, sessionId: string) => void;
  setPrDraft: (worktreePath: string, draft: PrDraft | null) => void;
  setAskPrompt: (kind: AskPrompt, text: string | null) => void;
  setWriterCommand: (command: string) => void;
  toggleChanges: () => void;
  setChangesScope: (scope: ChangesScope) => void;
  setPanelWidth: (panel: Panel, width: number) => void;
  setTheme: (theme: Theme) => void;
  setTerminalFontSize: (size: number) => void;
  setPanelFontSize: (size: number) => void;
  setDiffFontSize: (size: number) => void;
  setChatFontSize: (size: number) => void;
  setSidebarFontSize: (size: number) => void;
  setReopenSessions: (on: boolean) => void;
  setAgentView: (view: SessionKind) => void;
  setAutoUpdate: (on: boolean) => void;
  setShowCommits: (on: boolean) => void;
  setShowPullRequests: (on: boolean) => void;
  openDialog: (dialog: DialogState) => void;
  closeDialog: () => void;
}

export const useUi = create<UiState>()(
  persist(
    (set) => ({
      selectedProjectId: null,
      selectedWorktreePath: null,
      collapsed: {},
      activeSession: {},
      prDrafts: {},
      askPrompts: {},
      writerCommand: "",
      showChanges: true,
      changesScope: "uncommitted",
      panelWidths: {
        sidebar: PANEL_LIMITS.sidebar.initial,
        diff: PANEL_LIMITS.diff.initial,
        changes: PANEL_LIMITS.changes.initial,
      },
      theme: "system",
      terminalFontSize: 13,
      panelFontSize: 13,
      diffFontSize: 13,
      chatFontSize: 14,
      sidebarFontSize: 13,
      reopenSessions: true,
      agentView: "terminal",
      autoUpdate: true,
      showCommits: true,
      showPullRequests: true,
      dialog: null,
      select: (selectedProjectId, selectedWorktreePath) => set({ selectedProjectId, selectedWorktreePath }),
      toggleCollapsed: (id) => set((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } })),
      setActiveSession: (path, id) => set((s) => ({ activeSession: { ...s.activeSession, [path]: id } })),
      setPrDraft: (path, draft) =>
        set((s) => {
          const { [path]: _, ...rest } = s.prDrafts;
          return { prDrafts: draft ? { ...rest, [path]: draft } : rest };
        }),
      setAskPrompt: (kind, text) =>
        set((s) => {
          const { [kind]: _, ...rest } = s.askPrompts;
          return { askPrompts: text === null ? rest : { ...rest, [kind]: text } };
        }),
      setWriterCommand: (writerCommand) => set({ writerCommand }),
      toggleChanges: () => set((s) => ({ showChanges: !s.showChanges })),
      setChangesScope: (changesScope) => set({ changesScope }),
      setPanelWidth: (panel, width) => {
        const { min, max } = PANEL_LIMITS[panel];
        set((s) => ({ panelWidths: { ...s.panelWidths, [panel]: Math.round(Math.min(max, Math.max(min, width))) } }));
      },
      setTheme: (theme) => set({ theme }),
      setTerminalFontSize: (terminalFontSize) => set({ terminalFontSize }),
      setPanelFontSize: (panelFontSize) => set({ panelFontSize }),
      setDiffFontSize: (diffFontSize) => set({ diffFontSize }),
      setChatFontSize: (chatFontSize) => set({ chatFontSize }),
      setSidebarFontSize: (sidebarFontSize) => set({ sidebarFontSize }),
      setReopenSessions: (reopenSessions) => set({ reopenSessions }),
      setAgentView: (agentView) => set({ agentView }),
      setAutoUpdate: (autoUpdate) => set({ autoUpdate }),
      setShowCommits: (showCommits) => set({ showCommits }),
      setShowPullRequests: (showPullRequests) => set({ showPullRequests }),
      openDialog: (dialog) => set({ dialog }),
      closeDialog: () => set({ dialog: null }),
    }),
    {
      name: "worki-ui",
      partialize: ({
        selectedProjectId,
        selectedWorktreePath,
        collapsed,
        showChanges,
        changesScope,
        panelWidths,
        theme,
        terminalFontSize,
        panelFontSize,
        diffFontSize,
        chatFontSize,
        sidebarFontSize,
        reopenSessions,
        agentView,
        autoUpdate,
        showCommits,
        showPullRequests,
        prDrafts,
        askPrompts,
        writerCommand,
      }) => ({
        selectedProjectId,
        selectedWorktreePath,
        collapsed,
        showChanges,
        changesScope,
        panelWidths,
        theme,
        terminalFontSize,
        panelFontSize,
        diffFontSize,
        chatFontSize,
        sidebarFontSize,
        reopenSessions,
        agentView,
        autoUpdate,
        showCommits,
        showPullRequests,
        prDrafts,
        askPrompts,
        writerCommand,
      }),
    },
  ),
);
