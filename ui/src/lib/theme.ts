import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useSyncExternalStore } from "react";

import { useUi } from "./store";

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

/** Applies the chosen theme to the page and the window (title bar, vibrancy, system controls),
 * and the chosen font sizes. */
export function useApplyTheme() {
  const theme = useUi((s) => s.theme);
  const panelFont = useUi((s) => s.panelFontSize);
  const diffFont = useUi((s) => s.diffFontSize);
  const chatFont = useUi((s) => s.chatFontSize);
  const sidebarFont = useUi((s) => s.sidebarFontSize);
  useEffect(() => {
    const style = document.documentElement.style;
    style.setProperty("--panel-font", `${panelFont}px`);
    style.setProperty("--diff-font", `${diffFont}px`);
    style.setProperty("--chat-font", `${chatFont}px`);
    style.setProperty("--sidebar-font", `${sidebarFont}px`);
  }, [panelFont, diffFont, chatFont, sidebarFont]);
  useEffect(() => {
    const root = document.documentElement;
    if (theme === "system") delete root.dataset.theme;
    else root.dataset.theme = theme;
    getCurrentWindow()
      .setTheme(theme === "system" ? null : theme)
      .catch(() => {});
  }, [theme]);
}

const systemDark = () => darkQuery.matches;
const onSystemChange = (notify: () => void) => {
  darkQuery.addEventListener("change", notify);
  return () => darkQuery.removeEventListener("change", notify);
};

export function useIsDark(): boolean {
  const theme = useUi((s) => s.theme);
  const system = useSyncExternalStore(onSystemChange, systemDark);
  return theme === "dark" || (theme === "system" && system);
}
