import { useEffect } from "react";

import { api } from "./api";
import { useSessions } from "./queries";
import { useUi } from "./store";

/** Holds off sleep while the setting is on and an agent is working. */
export function useKeepAwake() {
  const enabled = useUi((s) => s.keepAwake);
  const { data: sessions = [] } = useSessions();
  const working = sessions.some((s) => s.state.kind === "running" && s.activity === "working");
  const on = enabled && working;
  useEffect(() => {
    api.keepAwake(on).catch(() => {});
  }, [on]);
}
