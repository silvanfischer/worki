export type DiffLine =
  | { kind: "hunk"; text: string }
  | { kind: "context" | "add" | "del"; text: string; oldNo: number | null; newNo: number | null }
  | { kind: "note"; text: string };

/** Parses `git diff` output for one file into display lines, skipping the file header. */
export function parseDiff(text: string): { lines: DiffLine[]; binary: boolean } {
  const lines: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  let inHunk = false;
  let binary = false;

  for (const raw of text.split("\n")) {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(raw);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      inHunk = true;
      lines.push({ kind: "hunk", text: raw });
      continue;
    }
    if (!inHunk) {
      if (raw.startsWith("Binary files")) binary = true;
      continue;
    }
    if (raw.startsWith("+")) lines.push({ kind: "add", text: raw.slice(1), oldNo: null, newNo: newNo++ });
    else if (raw.startsWith("-")) lines.push({ kind: "del", text: raw.slice(1), oldNo: oldNo++, newNo: null });
    else if (raw.startsWith(" ")) lines.push({ kind: "context", text: raw.slice(1), oldNo: oldNo++, newNo: newNo++ });
    else if (raw.startsWith("\\")) lines.push({ kind: "note", text: raw.slice(1).trim() });
  }
  return { lines, binary };
}

export type DiffRow = { kind: "line"; index: number } | { kind: "gap"; id: number; start: number; end: number };

/** Revealed lines of a collapsed run, counted from its top and from its bottom. */
export interface GapReveal {
  head: number;
  tail: number;
}

/**
 * Collapses unchanged lines further than `context` lines from a change, except those in `keep`.
 * Gap ids are the index of the run's first line, so they stay stable while lines are revealed.
 */
export function diffRows(
  lines: DiffLine[],
  revealed: Record<number, GapReveal>,
  keep: Set<number> = new Set(),
  context = 3,
): DiffRow[] {
  const shown = lines.map((l, i) => l.kind === "add" || l.kind === "del" || l.kind === "note" || keep.has(i));
  lines.forEach((l, i) => {
    if (l.kind !== "add" && l.kind !== "del") return;
    for (let j = Math.max(0, i - context); j <= Math.min(lines.length - 1, i + context); j++) shown[j] = true;
  });
  const rows: DiffRow[] = [];
  let i = 0;
  while (i < lines.length) {
    if (lines[i].kind === "hunk") {
      i++;
      continue;
    }
    if (shown[i]) {
      rows.push({ kind: "line", index: i });
      i++;
      continue;
    }
    const start = i;
    while (i < lines.length && !shown[i] && lines[i].kind === "context") i++;
    const end = i;
    const { head = 0, tail = 0 } = revealed[start] ?? {};
    if (head + tail >= end - start) {
      for (let j = start; j < end; j++) rows.push({ kind: "line", index: j });
      continue;
    }
    for (let j = start; j < start + head; j++) rows.push({ kind: "line", index: j });
    rows.push({ kind: "gap", id: start, start: start + head, end: end - tail });
    for (let j = end - tail; j < end; j++) rows.push({ kind: "line", index: j });
  }
  return rows;
}
