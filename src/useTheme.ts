import { useEffect, useLayoutEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { observeTheme, resolveTheme, type ThemePreference } from "./theme";

export function useTheme(preference: ThemePreference | undefined) {
  const [systemDark, setSystemDark] = useState(() => window.matchMedia("(prefers-color-scheme: dark)").matches);
  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const current = getCurrentWindow();
    return observeTheme(() => current.theme(), async update => {
      const onMedia = () => update(media.matches ? "dark" : "light");
      media.addEventListener("change", onMedia);
      let stopNative = () => {};
      try { stopNative = await current.onThemeChanged(({ payload }) => update(payload)); } catch { /* Media changes remain available. */ }
      return () => { media.removeEventListener("change", onMedia); stopNative(); };
    }, theme => setSystemDark(theme === "dark"));
  }, []);
  const resolved = resolveTheme(preference, systemDark);
  useLayoutEffect(() => {
    const root = document.documentElement;
    root.dataset.themeSwitching = "true";
    root.dataset.theme = resolved;
    root.dataset.themePreference = preference ?? "dark";
    let second = 0;
    const first = requestAnimationFrame(() => { second = requestAnimationFrame(() => { delete root.dataset.themeSwitching; }); });
    return () => { cancelAnimationFrame(first); cancelAnimationFrame(second); delete root.dataset.themeSwitching; };
  }, [resolved, preference]);
  return resolved;
}
