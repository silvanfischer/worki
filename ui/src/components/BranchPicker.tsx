import clsx from "clsx";
import { Check, ChevronDown, GitBranch, Search } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";

/** Every word typed appears in the branch name, in any order; empty matches everything. */
export function matchesBranch(branch: string, query: string): boolean {
  const name = branch.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean)
    .every((word) => name.includes(word));
}

/** Picks a branch from a long list: a button that opens a filterable list, local branches
 * first, then remote ones. Arrow keys move, Enter picks, Esc closes. */
export function BranchPicker({
  value,
  branches,
  onChange,
  disabled,
}: {
  value: string;
  branches: string[];
  onChange: (branch: string) => void;
  disabled?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [cursor, setCursor] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number; width: number; maxHeight: number } | null>(null);

  const groups = useMemo(() => {
    const matching = branches.filter((b) => matchesBranch(b, query));
    return [
      { label: "Local branches", items: matching.filter((b) => !b.startsWith("origin/")) },
      { label: "Remote branches", items: matching.filter((b) => b.startsWith("origin/")) },
    ].filter((g) => g.items.length > 0);
  }, [branches, query]);
  const flat = groups.flatMap((g) => g.items);

  useEffect(() => setCursor(Math.max(0, flat.indexOf(value))), [query, open]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => {
      const target = e.target as Node;
      if (!root.current?.contains(target) && !popup.current?.contains(target)) setOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [open]);

  // Drawn over everything (a dialog would clip it): under the field, as wide as it, down to the
  // window's bottom edge.
  useLayoutEffect(() => {
    if (!open) return;
    const measure = () => {
      const r = root.current?.getBoundingClientRect();
      if (r) setPlace({ left: r.left, top: r.bottom + 4, width: r.width, maxHeight: window.innerHeight - r.bottom - 16 });
    };
    measure();
    window.addEventListener("resize", measure);
    window.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", measure, true);
    };
  }, [open]);

  useEffect(() => {
    list.current?.querySelector(`[data-index="${cursor}"]`)?.scrollIntoView({ block: "nearest" });
  }, [cursor]);

  const pick = (branch: string) => {
    onChange(branch);
    setOpen(false);
    setQuery("");
  };

  let index = -1;
  return (
    <div ref={root} className="relative">
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
        className="flex h-8 w-full items-center gap-2 rounded-md border border-line bg-panel px-2.5 text-left text-[13px] hover:bg-hover disabled:opacity-50"
      >
        <GitBranch size={13} className="shrink-0 text-muted" />
        <span className="min-w-0 flex-1 truncate font-mono text-[12px]">{value}</span>
        <ChevronDown size={13} className="shrink-0 text-muted" />
      </button>
      {open && place && createPortal(
        <div
          ref={popup}
          style={{ left: place.left, top: place.top, width: place.width, maxHeight: Math.min(480, Math.max(160, place.maxHeight)) }}
          className="fixed z-[60] flex flex-col overflow-hidden rounded-lg border border-line bg-elevated shadow-xl"
        >
          <label className="flex items-center gap-2 border-b border-line px-2.5">
            <Search size={13} className="shrink-0 text-muted" />
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "ArrowDown") {
                  e.preventDefault();
                  setCursor((c) => Math.min(flat.length - 1, c + 1));
                } else if (e.key === "ArrowUp") {
                  e.preventDefault();
                  setCursor((c) => Math.max(0, c - 1));
                } else if (e.key === "Enter") {
                  e.preventDefault();
                  if (flat[cursor]) pick(flat[cursor]);
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  setOpen(false);
                }
              }}
              placeholder="Filter branches"
              spellCheck={false}
              className="h-8 min-w-0 flex-1 bg-transparent text-[13px] outline-none! placeholder:text-faint"
            />
          </label>
          <div ref={list} className="min-h-0 flex-1 overflow-y-auto py-1">
            {groups.length === 0 && <p className="px-3 py-2 text-xs text-faint">No branch matches “{query}”.</p>}
            {groups.map((g) => (
              <div key={g.label}>
                <div className="px-3 pt-1.5 pb-0.5 text-[10px] font-semibold tracking-wide text-faint uppercase">
                  {g.label}
                </div>
                {g.items.map((b) => {
                  index += 1;
                  const i = index;
                  return (
                    <button
                      type="button"
                      key={b}
                      data-index={i}
                      onMouseEnter={() => setCursor(i)}
                      onClick={() => pick(b)}
                      className={clsx(
                        "flex w-full items-center gap-2 px-3 py-1 text-left font-mono text-[12px]",
                        i === cursor ? "bg-selected text-fg" : "text-muted",
                      )}
                    >
                      <span className="min-w-0 flex-1 truncate">{b}</span>
                      {b === value && <Check size={12} className="shrink-0 text-accent" />}
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </div>,
        document.body,
      )}
    </div>
  );
}
