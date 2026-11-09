import { create } from "zustand";

export interface Toast {
  id: number;
  message: string;
  kind: "error" | "info";
}

interface ToastState {
  toasts: Toast[];
  push: (message: string, kind?: Toast["kind"]) => void;
  dismiss: (id: number) => void;
  /** Stops the toast from closing on its own, e.g. while it's being read or copied. */
  keep: (id: number) => void;
}

const timers = new Map<number, ReturnType<typeof setTimeout>>();

let next = 1;

export const useToasts = create<ToastState>()((set) => ({
  toasts: [],
  push: (message, kind = "error") => {
    const id = next++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { id, message, kind }] }));
    timers.set(
      id,
      setTimeout(() => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), kind === "error" ? 8000 : 4000),
    );
  },
  dismiss: (id) => {
    clearTimeout(timers.get(id));
    timers.delete(id);
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },
  keep: (id) => {
    clearTimeout(timers.get(id));
    timers.delete(id);
  },
}));

export const toast = (message: string, kind?: Toast["kind"]) => useToasts.getState().push(message, kind);
