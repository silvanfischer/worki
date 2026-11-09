import { create } from "zustand";
import { persist } from "zustand/middleware";

export type DialogState =
  | { kind: "newWorktree"; projectId: string }
  | { kind: "removeWorktree"; projectId: string; path: string }
  | { kind: "saveEnvironment"; projectId: string; path: string }
  | { kind: "projectSettings"; projectId: string }
  | { kind: "addRemoteProject" }
  | { kind: "settings"; tab: SettingsTab }
  | null;

export type SettingsTab = "general" | "agents" | "secrets" | "isolation" | "sshHosts" | "openIn";

export type ChangesScope = "uncommitted" | "branch";

export type Theme = "system" | "light" | "dark";

export const TERMINAL_FONT_SIZES = [11, 12, 13, 14, 15, 16, 18];

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
  showChanges: boolean;
  /** Whether the changes panel lists uncommitted changes or everything since the base branch. */
  changesScope: ChangesScope;
  panelWidths: Record<Panel, number>;
  theme: Theme;
  terminalFontSize: number;
  /** Start the Claude sessions that were open at the last quit again when worki starts. */
  reopenSessions: boolean;
  dialog: DialogState;
  select: (projectId: string, worktreePath: string | null) => void;
  toggleCollapsed: (projectId: string) => void;
  setActiveSession: (worktreePath: string, sessionId: string) => void;
  toggleChanges: () => void;
  setChangesScope: (scope: ChangesScope) => void;
  setPanelWidth: (panel: Panel, width: number) => void;
  setTheme: (theme: Theme) => void;
  setTerminalFontSize: (size: number) => void;
  setReopenSessions: (on: boolean) => void;
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
      showChanges: true,
      changesScope: "uncommitted",
      panelWidths: {
        sidebar: PANEL_LIMITS.sidebar.initial,
        diff: PANEL_LIMITS.diff.initial,
        changes: PANEL_LIMITS.changes.initial,
      },
      theme: "system",
      terminalFontSize: 13,
      reopenSessions: true,
      dialog: null,
      select: (selectedProjectId, selectedWorktreePath) => set({ selectedProjectId, selectedWorktreePath }),
      toggleCollapsed: (id) => set((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } })),
      setActiveSession: (path, id) => set((s) => ({ activeSession: { ...s.activeSession, [path]: id } })),
      toggleChanges: () => set((s) => ({ showChanges: !s.showChanges })),
      setChangesScope: (changesScope) => set({ changesScope }),
      setPanelWidth: (panel, width) => {
        const { min, max } = PANEL_LIMITS[panel];
        set((s) => ({ panelWidths: { ...s.panelWidths, [panel]: Math.round(Math.min(max, Math.max(min, width))) } }));
      },
      setTheme: (theme) => set({ theme }),
      setTerminalFontSize: (terminalFontSize) => set({ terminalFontSize }),
      setReopenSessions: (reopenSessions) => set({ reopenSessions }),
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
        reopenSessions,
      }) => ({
        selectedProjectId,
        selectedWorktreePath,
        collapsed,
        showChanges,
        changesScope,
        panelWidths,
        theme,
        terminalFontSize,
        reopenSessions,
      }),
    },
  ),
);
