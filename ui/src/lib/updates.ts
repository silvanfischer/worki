import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useEffect } from "react";
import { create } from "zustand";

import { errorMessage } from "./api";
import { useUi } from "./store";

/** How often worki looks for a new release while it runs. */
const CHECK_EVERY = 6 * 60 * 60 * 1000;
/** The first look waits a little, so it doesn't slow down starting. */
const FIRST_CHECK = 10_000;

interface UpdateState {
  /** A newer release, found on GitHub. */
  available: Update | null;
  status: "idle" | "checking" | "installing";
  /** Share of the download done, 0 to 1, while installing. */
  progress: number | null;
  /** What the last check or install said went wrong. */
  error: string | null;
  /** When the last check finished, in milliseconds. */
  checkedAt: number | null;
}

export const useUpdates = create<UpdateState>()(() => ({
  available: null,
  status: "idle",
  progress: null,
  error: null,
  checkedAt: null,
}));

/** Looks for a newer release. Returns whether one was found. */
export async function checkForUpdates(): Promise<boolean> {
  if (useUpdates.getState().status !== "idle") return !!useUpdates.getState().available;
  useUpdates.setState({ status: "checking", error: null });
  try {
    const update = await check();
    useUpdates.setState({ available: update, status: "idle", checkedAt: Date.now() });
    return !!update;
  } catch (e) {
    useUpdates.setState({ status: "idle", error: errorMessage(e), checkedAt: Date.now() });
    return false;
  }
}

/** Downloads and installs the available release (its signature checked against the key built
 * into worki), then restarts worki. */
export async function installUpdate() {
  const { available, status } = useUpdates.getState();
  if (!available || status !== "idle") return;
  useUpdates.setState({ status: "installing", progress: 0, error: null });
  let total = 0;
  let done = 0;
  try {
    await available.downloadAndInstall((event) => {
      if (event.event === "Started") total = event.data.contentLength ?? 0;
      else if (event.event === "Progress") {
        done += event.data.chunkLength;
        useUpdates.setState({ progress: total ? done / total : null });
      }
    });
    await relaunch();
  } catch (e) {
    useUpdates.setState({ status: "idle", progress: null, error: errorMessage(e) });
  }
}

/** Checks for updates soon after start and then every few hours, unless turned off. */
export function useAutoUpdates() {
  const enabled = useUi((s) => s.autoUpdate);
  useEffect(() => {
    if (!enabled) return;
    const first = setTimeout(checkForUpdates, FIRST_CHECK);
    const every = setInterval(checkForUpdates, CHECK_EVERY);
    return () => {
      clearTimeout(first);
      clearInterval(every);
    };
  }, [enabled]);
}
