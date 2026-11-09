/** App-wide actions that components carry out where their state lives (e.g. the session area). */
export type AppCommand =
  | { kind: "newSession"; profileId: string | null }
  | { kind: "nextTab"; step: 1 | -1 }
  | { kind: "closeTab" };

const EVENT = "worki:command";

export function runCommand(command: AppCommand) {
  window.dispatchEvent(new CustomEvent<AppCommand>(EVENT, { detail: command }));
}

export function onCommand(handler: (command: AppCommand) => void): () => void {
  const listener = (e: Event) => handler((e as CustomEvent<AppCommand>).detail);
  window.addEventListener(EVENT, listener);
  return () => window.removeEventListener(EVENT, listener);
}
