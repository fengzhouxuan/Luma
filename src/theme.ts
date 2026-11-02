export type ThemePreference = "dark" | "light" | "system";
export type ResolvedTheme = "dark" | "light";
export function resolveTheme(preference: ThemePreference | undefined, systemDark: boolean): ResolvedTheme {
  return preference === "system" ? systemDark ? "dark" : "light" : preference ?? "dark";
}

// Register before reading: a late initial read must not overwrite a newer event.
export function observeTheme(
  read: () => Promise<ResolvedTheme | null>,
  subscribe: (update: (theme: ResolvedTheme) => void) => Promise<() => void>,
  changed: (theme: ResolvedTheme) => void,
): () => void {
  let disposed = false; let revision = 0; let stop: (() => void) | undefined;
  const update = (theme: ResolvedTheme) => { if (!disposed) { revision++; changed(theme); } };
  const initial = revision;
  const readInitial = () => {
    if (disposed || revision !== initial) return;
    void read().then(theme => { if (!disposed && revision === initial && theme) update(theme); }).catch(() => {});
  };
  void subscribe(update).then(unlisten => {
    if (disposed) unlisten(); else { stop = unlisten; readInitial(); }
  }).catch(readInitial);
  return () => { disposed = true; stop?.(); };
}
