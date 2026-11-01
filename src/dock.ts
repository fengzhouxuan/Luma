import type { AppEntry, Config } from "./types";

// The dock deliberately ignores the current search and category. Missing entries
// stay in the persisted list and reappear when discovery finds them again.
export function fixedApps(apps: AppEntry[], config: Pick<Config, "pinned_apps" | "hidden_apps">): AppEntry[] {
  const byId = new Map(apps.map(app => [app.id, app]));
  const hidden = new Set(config.hidden_apps ?? []);
  return [...new Set(config.pinned_apps ?? [])]
    .filter(id => !hidden.has(id) && byId.has(id)).map(id => byId.get(id)!);
}

export function insertionBefore(items: { id: string; center: number }[], x: number, movingId: string): string | null {
  return items.find(item => item.id !== movingId && x < item.center)?.id ?? null;
}

export function adjacentBefore(items: AppEntry[], id: string, direction: -1 | 1): string | null | undefined {
  const index = items.findIndex(app => app.id === id);
  if (index < 0 || index + direction < 0 || index + direction >= items.length) return undefined;
  return direction === -1 ? items[index - 1].id : items[index + 2]?.id ?? null;
}
