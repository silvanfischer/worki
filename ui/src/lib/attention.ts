import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { useEffect } from "react";

import { useSessions } from "./queries";

/** A macOS notification, only while worki is in the background. */
export async function notify(title: string, body: string) {
  if (document.hasFocus()) return;
  let granted = await isPermissionGranted().catch(() => false);
  if (!granted) granted = (await requestPermission().catch(() => "denied")) === "granted";
  if (granted) sendNotification({ title, body });
}

/** Keeps the Dock badge at the number of agents waiting under Needs you. */
export function useAttention() {
  const { data: sessions = [] } = useSessions();
  const waiting = sessions.filter((s) => s.state.kind === "running" && s.activity === "waiting").length;
  useEffect(() => {
    getCurrentWindow()
      .setBadgeCount(waiting || undefined)
      .catch(() => {});
  }, [waiting]);
}
