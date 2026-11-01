import { pinyin } from "pinyin-pro";
import type { Usage } from "./types";

interface Searchable { id: string; name: string; path: string }
interface Preferences { pinned_apps?: string[]; usage?: Record<string, Usage>; aliases?: Record<string, string[]> }
const termsCache = new Map<string, { name: string; full: string; initials: string }>();
const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });
const normalize = (text: string) => text.toLowerCase().normalize("NFKD").replace(/\p{M}/gu, "").trim();

function terms(name: string) {
  let cached = termsCache.get(name);
  if (!cached) {
    const normalized = normalize(name);
    try {
      cached = { name: normalized,
        full: normalize(pinyin(name, { toneType: "none", type: "array" }).join("")).replace(/\s/g, ""),
        initials: normalize(pinyin(name, { pattern: "first", toneType: "none", type: "array" }).join("")).replace(/\s/g, ""),
      };
    } catch { cached = { name: normalized, full: normalized, initials: normalized }; }
    if (termsCache.size >= 4096) termsCache.clear();
    termsCache.set(name, cached);
  }
  return cached;
}

export function matchScore(name: string, query: string): number {
  const q = normalize(query);
  if (!q) return 0;
  const entry = terms(name);
  if (entry.name === q) return 0;
  if (entry.name.startsWith(q)) return 10;
  if (entry.name.includes(` ${q}`)) return 15;
  if (entry.name.includes(q)) return 20;
  if (q.split(/\s+/).length > 1 && q.split(/\s+/).every((word) => entry.name.includes(word))) return 25;
  const compact = q.replace(/\s/g, "");
  if (entry.full === compact) return 30;
  if (entry.full.startsWith(compact)) return 35;
  if (entry.full.includes(compact)) return 40;
  if (entry.initials === compact) return 45;
  if (entry.initials.startsWith(compact)) return 50;
  if (entry.initials.includes(compact)) return 55;
  let previous = -1;
  let gaps = 0;
  for (const character of compact) {
    const index = entry.name.indexOf(character, previous + 1);
    if (index < 0) return Infinity;
    gaps += index - previous - 1;
    previous = index;
  }
  return 90 + Math.min(gaps, 50);
}

export function rankApps<T extends Searchable>(apps: T[], query: string, preferences: Preferences = {}): T[] {
  const pins = new Set(preferences.pinned_apps ?? []);
  return apps.map((app) => ({ app, score: Math.min(matchScore(app.name, query), ...(!query.trim() ? [] : (preferences.aliases?.[app.id] ?? []).map((alias) => matchScore(alias, query) + 2))) }))
    .filter(({ score }) => Number.isFinite(score))
    .sort((a, b) => a.score - b.score || Number(pins.has(b.app.id)) - Number(pins.has(a.app.id)) ||
      (preferences.usage?.[b.app.id]?.count ?? 0) - (preferences.usage?.[a.app.id]?.count ?? 0) ||
      (preferences.usage?.[b.app.id]?.last_launch ?? 0) - (preferences.usage?.[a.app.id]?.last_launch ?? 0) ||
      collator.compare(a.app.name, b.app.name) || a.app.id.localeCompare(b.app.id))
    .map(({ app }) => app);
}
