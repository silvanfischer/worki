import { getCurrentWindow } from "@tauri-apps/api/window";
import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";
import { useEffect, useRef } from "react";

import { checksFailed, checksKey, useFailedChecks, usePullRequests } from "./pullRequests";
import { useSessions } from "./queries";
import { useUi } from "./store";

/** A macOS notification, only while worki is in the background. */
export async function notify(title: string, body: string) {
  if (document.hasFocus()) return;
  let granted = await isPermissionGranted().catch(() => false);
  if (!granted) granted = (await requestPermission().catch(() => "denied")) === "granted";
  if (granted) sendNotification({ title, body });
}

/** Keeps the Dock badge at the number of things under Needs you, and notifies when a pull request's checks fail. */
export function useAttention() {
  const { data: sessions = [] } = useSessions();
  const waiting = sessions.filter((s) => s.state.kind === "running" && s.activity === "waiting").length;
  const failed = useFailedChecks().length;
  useEffect(() => {
    getCurrentWindow()
      .setBadgeCount(waiting + failed || undefined)
      .catch(() => {});
  }, [waiting, failed]);

  const prs = usePullRequests();
  const keepDismissed = useUi((s) => s.keepDismissedChecks);
  const wasFailing = useRef(new Map<string, boolean>());
  const failing = prs.filter((p) => checksFailed(p.pr));
  const signature = prs.map((p) => `${checksKey(p)}:${checksFailed(p.pr)}`).join();
  useEffect(() => {
    keepDismissed(failing.map(checksKey));
    for (const p of prs) {
      const key = checksKey(p);
      const now = checksFailed(p.pr);
      if (now && wasFailing.current.get(key) === false) {
        notify(`Checks failed on #${p.pr.number}`, `${p.pr.title} · ${p.worktree.branch}`);
      }
      wasFailing.current.set(key, now);
    }
  }, [signature]);
}
