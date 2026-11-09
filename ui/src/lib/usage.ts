import type { ModelUsage } from "./api";

export interface UsageTotal {
  tokens: number;
  models: ModelUsage[];
}

export function totalUsage(usages: ModelUsage[]): UsageTotal | null {
  if (usages.length === 0) return null;
  const byModel = new Map<string, ModelUsage>();
  for (const u of usages) {
    const m = byModel.get(u.model);
    byModel.set(
      u.model,
      m
        ? {
            ...m,
            input: m.input + u.input,
            output: m.output + u.output,
            cacheWrite: m.cacheWrite + u.cacheWrite,
            cacheWrite1h: m.cacheWrite1h + u.cacheWrite1h,
            cacheRead: m.cacheRead + u.cacheRead,
          }
        : u,
    );
  }
  const models = [...byModel.values()];
  const tokens = models.reduce((n, m) => n + m.input + m.output + m.cacheWrite + m.cacheRead, 0);
  return { tokens, models };
}

export function formatTokens(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(n >= 1e4 ? 0 : 1)}k`;
  return String(n);
}

/** Multi-line breakdown for a tooltip. */
export function usageDetails(total: UsageTotal): string {
  return total.models
    .map(
      (m) =>
        `${m.model}: ${formatTokens(m.input)} in, ${formatTokens(m.output)} out, ${formatTokens(m.cacheWrite)} cache write, ${formatTokens(m.cacheRead)} cache read`,
    )
    .join("\n");
}
