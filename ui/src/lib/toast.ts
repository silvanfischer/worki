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
}

let next = 1;

export const useToasts = create<ToastState>()((set) => ({
  toasts: [],
  push: (message, kind = "error") => {
    const id = next++;
    set((s) => ({ toasts: [...s.toasts.slice(-3), { id, message, kind }] }));
    setTimeout(() => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })), kind === "error" ? 8000 : 4000);
  },
  dismiss: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));

export const toast = (message: string, kind?: Toast["kind"]) => useToasts.getState().push(message, kind);
