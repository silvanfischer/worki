import clsx from "clsx";
import { X } from "lucide-react";
import {
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
  createContext,
  useContext,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";

import type { LogLine } from "../lib/api";

/**
 * Inside a page (the Settings window) a `Dialog` renders as a plain section: title as heading,
 * body, then its buttons at the bottom; no overlay, no Escape handling.
 */
export const EmbeddedDialog = createContext(false);

type Variant = "primary" | "secondary" | "ghost" | "danger";

export function Button({
  variant = "secondary",
  className,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      {...props}
      className={clsx(
        "inline-flex h-7 items-center justify-center gap-1.5 rounded-md px-3 text-[13px] font-medium transition-colors disabled:opacity-40",
        variant === "primary" && "bg-accent text-accent-fg hover:brightness-110",
        variant === "secondary" && "border border-line bg-elevated text-fg shadow-xs hover:bg-hover",
        variant === "ghost" && "text-muted hover:bg-hover hover:text-fg",
        variant === "danger" && "bg-removed text-white hover:brightness-110",
        className,
      )}
    />
  );
}

export function IconButton({
  className,
  label,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      {...props}
      title={label}
      aria-label={label}
      className={clsx(
        "inline-flex size-6 shrink-0 items-center justify-center rounded-md text-muted transition-colors hover:bg-hover hover:text-fg",
        className,
      )}
    />
  );
}

const fieldClass =
  "w-full rounded-md border border-line bg-panel px-2.5 text-[13px] text-fg placeholder:text-faint focus:border-accent focus:outline-none";

export function Input({ className, ...props }: InputHTMLAttributes<HTMLInputElement>) {
  return <input {...props} spellCheck={false} autoCorrect="off" className={clsx(fieldClass, "h-8", className)} />;
}

/** `maxRows` makes it grow with its text from `rows` up to that many lines, then scroll. */
export function Textarea({
  className,
  maxRows,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & { maxRows?: number }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || !maxRows) return;
    const style = getComputedStyle(el);
    const line = parseFloat(style.lineHeight) || 16;
    const chrome =
      parseFloat(style.paddingTop) + parseFloat(style.paddingBottom) + parseFloat(style.borderTopWidth) * 2;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight + parseFloat(style.borderTopWidth) * 2, line * maxRows + chrome)}px`;
  }, [props.value, maxRows]);
  return (
    <textarea
      ref={ref}
      {...props}
      spellCheck={false}
      className={clsx(fieldClass, "py-2 font-mono text-xs", maxRows && "resize-none", className)}
    />
  );
}

export function Select({ className, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select
      {...props}
      className={clsx("rounded-md border border-line bg-panel px-2 focus:border-accent focus:outline-none", className)}
    />
  );
}

export function Field({ label, hint, children }: { label: string; hint?: ReactNode; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-xs font-medium text-muted">{label}</span>
      {children}
      {hint && <span className="text-xs text-faint">{hint}</span>}
    </label>
  );
}

export function Checkbox({
  checked,
  onChange,
  disabled,
  children,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  children: ReactNode;
}) {
  return (
    <label className={clsx("flex items-start gap-2", disabled && "opacity-50")}>
      <input
        type="checkbox"
        className="mt-0.5 accent-[var(--accent)]"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span>{children}</span>
    </label>
  );
}

export function Dialog({
  title,
  onClose,
  children,
  footer,
  width = 460,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  const embedded = useContext(EmbeddedDialog);
  useEffect(() => {
    if (embedded) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, embedded]);

  if (embedded) {
    return (
      <section aria-label={title} className="flex min-h-0 flex-1 flex-col">
        <h2 className="px-6 pt-5 pb-3 text-[15px] font-semibold">{title}</h2>
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-6">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-line px-6 py-3">{footer}</div>}
      </section>
    );
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/25 pt-[12vh] backdrop-blur-[1px]"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div
        role="dialog"
        aria-label={title}
        style={{ width }}
        className="flex max-h-[76vh] flex-col overflow-hidden rounded-xl border border-line bg-elevated shadow-2xl"
      >
        <div className="flex items-center justify-between px-4 pt-3.5 pb-2">
          <h2 className="text-[14px] font-semibold">{title}</h2>
          <IconButton label="Close" onClick={onClose}>
            <X size={14} />
          </IconButton>
        </div>
        <div className="flex flex-col gap-4 overflow-y-auto px-4 pt-1 pb-4">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-line px-4 py-3">{footer}</div>}
      </div>
    </div>
  );
}

interface MenuItem {
  label: string;
  icon?: ReactNode;
  danger?: boolean;
  onSelect: () => void;
}

export function Menu({ trigger, items }: { trigger: (open: () => void) => ReactNode; items: MenuItem[] }) {
  const [isOpen, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!isOpen) return;
    const close = (e: MouseEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [isOpen]);

  return (
    <div ref={ref} className="relative">
      {trigger(() => setOpen((o) => !o))}
      {isOpen && (
        <div className="absolute right-0 z-40 mt-1 min-w-48 rounded-lg border border-line bg-elevated p-1 shadow-xl">
          {items.map((item) => (
            <button
              key={item.label}
              onClick={(e) => {
                e.stopPropagation();
                setOpen(false);
                item.onSelect();
              }}
              className={clsx(
                "flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-[13px] hover:bg-accent hover:text-accent-fg",
                item.danger && "text-removed",
              )}
            >
              <span className="flex w-4 justify-center">{item.icon}</span>
              {item.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function ErrorText({ children }: { children: ReactNode }) {
  if (!children) return null;
  return (
    <div className="selectable rounded-md border border-removed/30 bg-removed/10 px-3 py-2 text-xs whitespace-pre-wrap text-removed">
      {children}
    </div>
  );
}

export function WarnText({ children }: { children: ReactNode }) {
  return <p className="rounded-md border border-warn/30 bg-warn/10 px-3 py-2 text-xs text-warn">{children}</p>;
}

export function DiffStat({
  insertions,
  deletions,
  className,
}: {
  insertions: number;
  deletions: number;
  className?: string;
}) {
  return (
    <span className={clsx("font-mono text-[11px]", className)}>
      <span className="text-added">+{insertions}</span> <span className="text-removed">−{deletions}</span>
    </span>
  );
}

/** Streamed command output that keeps scrolled to the newest line. */
export function LogView({ lines, className }: { lines: LogLine[]; className?: string }) {
  const end = useRef<HTMLDivElement>(null);
  useEffect(() => end.current?.scrollIntoView({ block: "end" }), [lines]);
  return (
    <div
      className={clsx(
        "selectable overflow-y-auto rounded-md border border-line p-2 font-mono text-[11px] leading-relaxed",
        className,
      )}
    >
      {lines.map((l, i) => (
        <div
          key={i}
          className={clsx(
            "break-all whitespace-pre-wrap",
            l.stream === "info" && "text-accent",
            l.stream === "stderr" && "text-muted",
          )}
        >
          {l.line}
        </div>
      ))}
      <div ref={end} />
    </div>
  );
}
