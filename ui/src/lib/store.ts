import { create } from "zustand";
import { persist } from "zustand/middleware";

export type DialogState =
  | { kind: "newWorktree"; projectId: string }
  | { kind: "removeWorktree"; projectId: string; path: string }
  | { kind: "projectSettings"; projectId: string }
  | { kind: "addRemoteProject" }
  | { kind: "settings"; tab: SettingsTab }
  | null;

export type SettingsTab = "agents" | "secrets" | "isolation" | "openIn";

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
  panelWidths: Record<Panel, number>;
  dialog: DialogState;
  select: (projectId: string, worktreePath: string | null) => void;
  toggleCollapsed: (projectId: string) => void;
  setActiveSession: (worktreePath: string, sessionId: string) => void;
  toggleChanges: () => void;
  setPanelWidth: (panel: Panel, width: number) => void;
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
      panelWidths: {
        sidebar: PANEL_LIMITS.sidebar.initial,
        diff: PANEL_LIMITS.diff.initial,
        changes: PANEL_LIMITS.changes.initial,
      },
      dialog: null,
      select: (selectedProjectId, selectedWorktreePath) => set({ selectedProjectId, selectedWorktreePath }),
      toggleCollapsed: (id) => set((s) => ({ collapsed: { ...s.collapsed, [id]: !s.collapsed[id] } })),
      setActiveSession: (path, id) => set((s) => ({ activeSession: { ...s.activeSession, [path]: id } })),
      toggleChanges: () => set((s) => ({ showChanges: !s.showChanges })),
      setPanelWidth: (panel, width) => {
        const { min, max } = PANEL_LIMITS[panel];
        set((s) => ({ panelWidths: { ...s.panelWidths, [panel]: Math.round(Math.min(max, Math.max(min, width))) } }));
      },
      openDialog: (dialog) => set({ dialog }),
      closeDialog: () => set({ dialog: null }),
    }),
    {
      name: "worki-ui",
      partialize: ({ selectedProjectId, selectedWorktreePath, collapsed, showChanges, panelWidths }) => ({
        selectedProjectId,
        selectedWorktreePath,
        collapsed,
        showChanges,
        panelWidths,
      }),
    },
  ),
);
