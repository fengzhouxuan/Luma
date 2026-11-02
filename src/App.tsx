import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { rankApps } from "./search";
import { DiagnosticsPanel } from "./DiagnosticsPanel";
import { AppEditor, type EditorTarget } from "./AppEditor";
import { AppMenu } from "./AppMenu";
import { RunningAppDialog, type RunningWindows } from "./RunningAppDialog";
import { EntryIcon } from "./EntryIcon";
import { HealthPanel } from "./HealthPanel";
import { dockShortcutIndex } from "./dockShortcuts";
import { UiIcon, CategoryIcon } from "./UiIcon";
import { Dialog } from "./Dialog";
import { useOverlayFocus } from "./overlayFocus";
import { LatestWriter } from "./latestWriter";
import { FixedBar } from "./FixedBar";
import { FileSearch, type FileSearchHandle } from "./FileSearchPanel";
import { fixedApps } from "./dock";
import { useAppDrag } from "./useAppDrag";
import { ManagementTools, ImportPreview } from "./ManagementTools";
import type { AppEntry, Config, HotkeyConfig, Usage, ScanResult, Diagnostics, ManualApp, AutostartStatus, BackupPreview, EntryHealth } from "./types";
import { IconLoader, iconKey, type IconTarget } from "./iconLoader";
import "./App.css";
import "./layout.css";

const AUTO_SCAN_INTERVAL = 5 * 60 * 1000;
const WAKE_SCAN_INTERVAL = 60 * 1000;

function hotkeyLabel(h: HotkeyConfig): string {
  const modNames: Record<string, string> = { ctrl: "Ctrl", alt: "Alt", shift: "Shift", meta: "Win" };
  const mods = h.modifiers.map((m) => modNames[m] ?? m).join(" + ");
  const keyName = h.key.length === 1 ? h.key.toUpperCase() : h.key;
  return `${mods}${mods ? " + " : ""}${keyName}`;
}

function App() {
  const [apps, setApps] = useState<AppEntry[]>([]);
  const [query, setQuery] = useState("");
  const [searchMode, setSearchMode] = useState<"apps" | "files">("apps");
  const [composing, setComposing] = useState(false);
  const fileSearchRef = useRef<FileSearchHandle>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [iconRevision, setIconRevision] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [healthOpen, setHealthOpen] = useState(false);
  const [healthChecks, setHealthChecks] = useState<EntryHealth[]>([]);
  const [healthBusy, setHealthBusy] = useState(false);
  const [healthError, setHealthError] = useState("");
  const [altPressed, setAltPressed] = useState(false);
  const launchPending = useRef(false);
  const checkingLaunch = useRef(false);
  const launchRevision = useRef(0);
  const [runningChoice, setRunningChoice] = useState<{ app: AppEntry; info: RunningWindows } | null>(null);
  const healthById = useMemo(() => new Map(healthChecks.filter(item=>item.state!=="unchecked").map(item=>[item.id,item])),[healthChecks]);
  const [diagnosticsOpen, setDiagnosticsOpen] = useState(false);
  const [diagnosticId, setDiagnosticId] = useState<string | null>(null);
  const [snapshot, setSnapshot] = useState<Diagnostics | null>(null);
  const [config, setConfig] = useState<Config | null>(null);
  const [editor, setEditor] = useState<EditorTarget | null>(null);
  const [menu, setMenu] = useState<{ app: AppEntry; x: number; y: number } | null>(null);
  const [preview, setPreview] = useState<BackupPreview | null>(null);
  const [autostart, setAutostart] = useState<AutostartStatus | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);
  const [notice, setNotice] = useState("");
  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => setNotice(""), 6000);
    return () => window.clearTimeout(timer);
  }, [notice]);
  const [capturing, setCapturing] = useState(false);
  const [hotkeyDraft, setHotkeyDraft] = useState<HotkeyConfig | null>(null);
  const [extraDirDraft, setExtraDirDraft] = useState("");
  const [activeCategory, setActiveCategory] = useState<string>("全部");
  const [categoryDraft, setCategoryDraft] = useState("");
  const rootRef = useRef<HTMLDivElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const overlayKey = runningChoice ? `running:${runningChoice.app.id}` : menu ? `menu:${menu.app.id}` : preview ? "import" : editor ? "editor" : healthOpen ? "health" : diagnosticsOpen ? "diagnostics" : settingsOpen ? "settings" : null;
  useOverlayFocus(rootRef, overlayKey, openerRef, inputRef);
  const appearanceWriter = useRef<LatestWriter<Config["appearance"]> | null>(null);
  if (!appearanceWriter.current) appearanceWriter.current = new LatestWriter(
    appearance => invoke("set_appearance", { appearance }), error => setNotice(`保存外观失败：${error}`));
  useEffect(() => () => { void appearanceWriter.current?.flush(); }, []);
  const changeAppearance = (patch: Partial<Config["appearance"]>) => {
    if (!config) return;
    const appearance = { ...config.appearance, ...patch };
    setConfig(current => current ? { ...current, appearance } : current);
    appearanceWriter.current!.set(appearance);
  };
  const flushAppearance = () => { void appearanceWriter.current?.flush(); };
  const gridRef = useRef<HTMLDivElement>(null);
  const dockRef = useRef<HTMLDivElement>(null);
  const categoriesRef = useRef<HTMLDivElement>(null);
  const pinQueue = useRef<Promise<void>>(Promise.resolve());
  const mutatePins = (command: string, args: Record<string, unknown>) => {
    const job = pinQueue.current.then(async () => {
      const next = await invoke<Config>(command, args);
      setConfig(current => current ? { ...current, pinned_apps: next.pinned_apps } : next);
    });
    pinQueue.current = job.catch(error => setNotice(`保存固定应用失败：${error}`));
    return pinQueue.current;
  };
  const placePinned = (app: AppEntry, beforeId: string | null) => { void mutatePins("place_pinned_app", { appId: app.id, beforeId }); };
  const appDrag = useAppDrag(dockRef, categoriesRef, placePinned, (app, category) => {
    void setAppCategory(app.id, category).then(saved => {
      if (saved) setNotice(category ? `已将「${app.name}」移到「${category}」` : `「${app.name}」已恢复自动分类`);
    });
  });
  const dragging = !!appDrag.drag;
  const scanInFlight = useRef<Promise<void> | null>(null);
  const scanPending = useRef(false);
  const forcePending = useRef(false);
  const lastScanAt = useRef<number | null>(null);
  const catalogRevision = useRef<number | null>(null);
  const iconLoader = useRef<IconLoader | null>(null);
  if (!iconLoader.current) {
    iconLoader.current = new IconLoader(
      (app) => invoke<string | null>("get_icon", {
        path: app.path,
        shortcutPath: app.shortcut_path ?? null,
      }),
      (key, icon) => setApps((prev) => prev.map((app) =>
        iconKey(app) === key && app.icon !== icon ? { ...app, icon } : app
      )),
    );
  }

  useEffect(() => () => iconLoader.current?.reset(), []);

  useEffect(() => { void invoke("set_overlay_open", { open: !!overlayKey || dragging }); }, [overlayKey, dragging]);
  useEffect(() => {
    const down=(event: KeyboardEvent)=>{ if(event.key==="Alt"&&!event.repeat&&!overlayKey)setAltPressed(true); };
    const up=(event: KeyboardEvent)=>{ if(event.key==="Alt")setAltPressed(false); };
    const clear=()=>setAltPressed(false);
    window.addEventListener("keydown",down);window.addEventListener("keyup",up);window.addEventListener("blur",clear);
    if(overlayKey)clear();
    return()=>{window.removeEventListener("keydown",down);window.removeEventListener("keyup",up);window.removeEventListener("blur",clear);};
  },[overlayKey]);
  useEffect(() => () => { void invoke("set_overlay_open", { open: false }); }, []);
  useEffect(() => {
    let disposed = false; let stop: (() => void) | undefined;
    void getCurrentWindow().onDragDropEvent(({ payload }) => {
      if (payload.type !== "drop" || disposed) return;
      if (payload.paths.length !== 1) { setNotice("请每次拖入一个应用、文件夹或脚本"); return; }
      void invoke<ManualApp>("prepare_manual_app", { path: payload.paths[0] }).then(manual => {
        if (!disposed) { setSettingsOpen(false); setDiagnosticsOpen(false); setEditor({ manual }); }
      }).catch(error => { if (!disposed) setNotice(String(error)); });
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; }).catch(error => console.error("监听文件拖入失败", error));
    return () => { disposed = true; stop?.(); };
  }, []);

  const addManualApp = async () => {
    setSettingsOpen(false); setDiagnosticsOpen(false);
    setEditor({ manual: { kind: "app", id: `manual-${crypto.randomUUID()}`, name: "", path: "", arguments: "", working_dir: null, shortcut_path: null } });
  };
  const personalSaved = (next: Config) => { setConfig(next); setEditor(null); setPreview(null); void scan(false, true); };
  const recheckHealth = async () => {
    setHealthBusy(true); setHealthError("");
    try { setHealthChecks(await invoke<EntryHealth[]>("check_entry_health")); }
    catch(error) { setHealthError(String(error)); }
    finally { setHealthBusy(false); }
  };
  const openHealth = () => { setSettingsOpen(false);setDiagnosticsOpen(false);setHealthOpen(true);void recheckHealth(); };
  const repairEntry = (app: AppEntry) => { setHealthOpen(false);setMenu(null);setEditor({repair:app}); };

  // 共用一个扫描任务；目录变更发生在扫描中时，再补扫一次。
  const scan = useCallback((rescan = false, changed = false): Promise<void> => {
    if (scanInFlight.current) {
      if (rescan || changed) scanPending.current = true;
      if (rescan) forcePending.current = true;
      return scanInFlight.current;
    }
    setRefreshing(true);
    const task = Promise.resolve().then(async () => {
      let force = rescan;
      try {
        do {
          scanPending.current = false;
          try {
            const result = await invoke<ScanResult>("scan_apps", { force });
            setHealthChecks(result.health ?? []);
            if (catalogRevision.current !== result.report.revision) {
              catalogRevision.current = result.report.revision;
              iconLoader.current?.reset();
              setApps((prev) => {
                const icons = new Map(prev.map((app) => [iconKey(app), app.icon]));
                return result.apps.map((app) => ({ ...app, icon: icons.get(iconKey(app)) ?? app.icon }));
              });
              setIconRevision((revision) => revision + 1);
            }
            setScanError(null);
          } catch (e) {
            console.error("扫描失败", e);
            setScanError("扫描失败，请点击刷新重试");
          }
          force = forcePending.current;
          forcePending.current = false;
        } while (scanPending.current);
      } finally {
        lastScanAt.current = Date.now();
        scanInFlight.current = null;
        setLoading(false);
        setRefreshing(false);
      }
    });
    scanInFlight.current = task;
    return task;
  }, []);

  // 加载配置
  const loadConfig = useCallback(async () => {
    try {
      const cfg = await invoke<Config>("get_config");
      setConfig(cfg);
    } catch (e) {
      console.error("加载配置失败", e);
    }
  }, []);

  const refreshDiagnostics = useCallback(async () => {
    try { setSnapshot(await invoke<Diagnostics>("get_diagnostics")); }
    catch (error) { console.error("读取诊断失败", error); }
  }, []);
  const openDiagnostics = (id: string | null) => {
    setDiagnosticId(id); setDiagnosticsOpen(true); setSettingsOpen(false);
    void refreshDiagnostics();
  };

  useEffect(() => { if (settingsOpen) void refreshDiagnostics(); }, [settingsOpen, refreshDiagnostics]);
  useEffect(() => { if (settingsOpen) void invoke<AutostartStatus>("get_autostart_status").then(setAutostart).catch(error => setNotice(String(error))); }, [settingsOpen]);

  useEffect(() => {
    void scan();
    void loadConfig();
    void refreshDiagnostics();
    const focusTimer = setTimeout(() => inputRef.current?.focus(), 100);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const listeners: (() => void)[] = [];
    const subscribe = (name: string, handler: () => void) => {
      void listen(name, () => { if (!disposed) handler(); }).then((stop) => {
        if (disposed) stop(); else listeners.push(stop);
      }).catch((error) => console.error(`监听 ${name} 失败`, error));
    };
    subscribe("apps-changed", () => void scan(false, true));
    subscribe("refresh-apps", () => void scan(true));
    subscribe("open-settings", () => { setDiagnosticsOpen(false); setSettingsOpen(true); });
    subscribe("open-diagnostics", () => openDiagnostics(null));
    void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (!focused && checkingLaunch.current) ++launchRevision.current;
      if (!disposed && focused &&
        (lastScanAt.current === null || Date.now() - lastScanAt.current >= WAKE_SCAN_INTERVAL)) {
        void scan();
      }
    }).then((stop) => {
      if (disposed) stop();
      else unlisten = stop;
    }).catch((e) => console.error("监听窗口唤醒失败", e));
    return () => {
      disposed = true;
      clearTimeout(focusTimer);
      unlisten?.();
      listeners.forEach((stop) => stop());
    };
  }, [scan, loadConfig, refreshDiagnostics]);

  // 每次扫描完成后重新计时，唤醒刷新不会与旧定时任务挤在一起。
  useEffect(() => {
    if (refreshing) return;
    const timer = window.setTimeout(() => void scan(), AUTO_SCAN_INTERVAL);
    return () => window.clearTimeout(timer);
  }, [refreshing, scan]);

  // 全局监听 Esc：无论焦点落在哪个元素（输入框、复选框、甚至 body），
  // 都能可靠关闭窗口，避免依赖 DOM 冒泡导致的 Esc 失效。
  useEffect(() => {
    const onGlobalKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.repeat) return;
      e.preventDefault();
      hideLauncher();
    };
    window.addEventListener("keydown", onGlobalKeyDown);
    return () => window.removeEventListener("keydown", onGlobalKeyDown);
  }, [settingsOpen, diagnosticsOpen, healthOpen, capturing, editor, preview, menu, runningChoice, searchMode]);

  // 应用外观配置到 CSS 变量
  useEffect(() => {
    if (!config) return;
    const root = document.documentElement;
    root.style.setProperty("--mv-launchpad-opacity", String(config.appearance.opacity));
    // With native backdrop disabled, keep sharp desktop text from showing through.
    root.style.setProperty("--mv-launchpad-tint", String(0.985 + config.appearance.opacity * 0.015));
    root.style.setProperty("--mv-glass-cloud", String(0.06 + config.appearance.opacity * 0.18));
    root.style.setProperty("--mv-glass-cyan", String(0.02 + config.appearance.opacity * 0.1));
    root.style.setProperty("--mv-launchpad-radius", `${config.appearance.corner_radius}px`);
  }, [config]);

  // 保存配置
  const saveConfig = async (cfg: Config) => {
    try {
      await invoke("set_config", { cfg });
      setConfig((current) => current ? { ...current, appearance: cfg.appearance, hotkey: cfg.hotkey } : cfg);
    } catch (e) {
      console.error("保存配置失败", e);
      alert(`保存配置失败: ${e}`);
    }
  };

  // 保存额外扫描目录列表
  const saveExtraDirs = async (dirs: string[]) => {
    try {
      await invoke("set_extra_dirs", { dirs });
      setConfig((c) => (c ? { ...c, extra_dirs: dirs } : c));
      // 目录变更后重新扫描应用
      void scan(true);
    } catch (e) {
      console.error("保存额外目录失败", e);
      alert(`保存额外目录失败: ${e}`);
    }
  };

  // 启动应用
  const finishLaunch = async () => {
    await invoke("hide_window");
    setRunningChoice(null);
    setQuery("");
  };
  const launch = async (app: AppEntry, admin = false, bypass = false, newWindow = false) => {
    if(launchPending.current)return;
    launchPending.current=true;
    const revision = ++launchRevision.current;
    try {
      if (!admin && !bypass && (!app.kind || app.kind === "app")) {
        checkingLaunch.current = true;
        const info = await invoke<RunningWindows>("get_running_windows", { appId: app.id });
        checkingLaunch.current = false;
        if (revision !== launchRevision.current) return;
        if (info.windows.length) { setRunningChoice({ app, info }); return; }
      }
      const result = await invoke<{ usage: Usage | null }>("launch_app", { appId: app.id, admin, newWindow });
      if (result.usage) setConfig((current) => current ? { ...current, usage: { ...current.usage, [app.id]: result.usage! } } : current);
      await finishLaunch();
    } catch (e) {
      if (bypass && runningChoice) throw e;
      if (revision === launchRevision.current) setNotice(`启动失败：${e}`);
    } finally {
      checkingLaunch.current=false;
      launchPending.current=false;
    }
  };
  const activateWindow = async (app: AppEntry, windowId: string) => {
    if (launchPending.current) return;
    launchPending.current = true;
    try {
      await invoke("activate_app_window", { appId: app.id, windowId });
      await finishLaunch();
    } finally { launchPending.current = false; }
  };

  // 手动设置应用分类
  const setAppCategory = async (appId: string, category: string) => {
    try {
      await invoke("set_app_category", { appId, category });
      setConfig((c) => {
        if (!c) return c;
        const appCategories = { ...(c.app_categories ?? {}) };
        appCategories[appId] = category;
        return { ...c, app_categories: appCategories };
      });
      return true;
    } catch (e) {
      console.error("设置分类失败", e);
      setNotice(`设置分类失败: ${e}`);
      return false;
    }
  };

  const togglePinned = async (app: AppEntry) => {
    await mutatePins("set_app_pinned", { appId: app.id, pinned: !config?.pinned_apps?.includes(app.id) });
  };
  const toggleHidden = async (app: AppEntry) => {
    try { setConfig(await invoke<Config>("set_app_hidden", { appId: app.id, hidden: !config?.hidden_apps?.includes(app.id) })); }
    catch (error) { alert(`设置隐藏失败: ${error}`); }
  };

  // 添加自定义分类
  const addCustomCategory = async (name: string) => {
    try {
      await invoke("add_custom_category", { name });
      setConfig((c) =>
        c
          ? { ...c, custom_categories: [...new Set([...(c.custom_categories ?? []), name])] }
          : c
      );
    } catch (e) {
      console.error("添加分类失败", e);
      alert(`添加分类失败: ${e}`);
    }
  };

  // 删除自定义分类
  const removeCustomCategory = async (name: string) => {
    try {
      await invoke("remove_custom_category", { name });
      setConfig((c) => {
        if (!c) return c;
        const appCategories = { ...(c.app_categories ?? {}) };
        Object.keys(appCategories).forEach((k) => {
          if (appCategories[k] === name) delete appCategories[k];
        });
        return {
          ...c,
          custom_categories: (c.custom_categories ?? []).filter((x) => x !== name),
          app_categories: appCategories,
        };
      });
      if (activeCategory === name) setActiveCategory("全部");
      // 显示分类由配置映射派生，删除映射后立即恢复自动分类。
    } catch (e) {
      console.error("删除分类失败", e);
      alert(`删除分类失败: ${e}`);
    }
  };

  // 关闭启动器（含设置面板）
  const hideLauncher = () => {
    if (!overlayKey && searchMode === "files" && fileSearchRef.current?.closePreview()) return;
    if (launchPending.current && !checkingLaunch.current) return;
    ++launchRevision.current;
    if (runningChoice) { setRunningChoice(null); return; }
    if (preview) { setPreview(null); return; }
    if (editor) { setEditor(null); return; }
    if (menu) { setMenu(null); return; }
    if (diagnosticsOpen) { setDiagnosticsOpen(false); return; }
    if (healthOpen) { setHealthOpen(false); return; }
    if (settingsOpen) {
      setSettingsOpen(false);
      setCapturing(false);
      // useOverlayFocus restores the opener after the panel leaves the DOM.
      return;
    }
    invoke("hide_window");
    setQuery("");
  };

  // 键盘导航：方向键 + 回车启动 + Ctrl+回车管理员 + Esc 隐藏
  const handleKeyDown = (e: React.KeyboardEvent) => {
    // 快捷键捕获模式：拦截所有按键，解析成快捷键
    if (capturing) {
      e.preventDefault();
      const parsed = parseHotkey(e);
      if (parsed) {
        setHotkeyDraft(parsed);
      }
      return;
    }

    if (e.key === "Escape") {
      // Esc 已由全局 window 监听统一处理，这里 stopPropagation 防止重复触发
      e.stopPropagation();
      if (e.repeat) return;
      hideLauncher();
      return;
    }

    // 设置面板打开时，不处理网格导航
    if (overlayKey || dragging) return;

    if (searchMode === "files" && fileSearchRef.current?.onKeyDown(e)) return;

    // 搜索框以外的输入控件保留自己的键盘行为。
    const target = e.target;
    if (
      target instanceof HTMLElement &&
      target !== inputRef.current &&
      (target.isContentEditable || target.closest("input, textarea, select, button"))
    ) {
      return;
    }

    if (searchMode === "files") return;
    if (e.nativeEvent.isComposing) return;
    const cols = getColumnCount();
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + cols, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - cols, 0));
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowLeft") {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const app = filtered[selectedIndex];
      if (app) {
        void launch(app, e.ctrlKey || e.metaKey, e.shiftKey);
      }
    }
  };

  // 解析键盘事件为 HotkeyConfig（用于捕获模式）
  const parseHotkey = (e: React.KeyboardEvent): HotkeyConfig | null => {
    const isModifier = e.key === "Control" || e.key === "Alt" || e.key === "Shift" || e.key === "Meta";
    if (isModifier) return null; // 只按修饰键不算

    const mods: string[] = [];
    if (e.ctrlKey) mods.push("ctrl");
    if (e.altKey) mods.push("alt");
    if (e.shiftKey) mods.push("shift");
    if (e.metaKey) mods.push("meta");

    const key = normalizeKey(e.key);
    if (!key) return null;

    // 必须至少有一个修饰键（RegisterHotKey 只支持组合键）
    if (mods.length === 0) return null;

    // 主键不能是修饰键
    if (key === "alt" || key === "ctrl" || key === "shift" || key === "meta") {
      return null;
    }
    return { mode: "combo", modifiers: mods, key };
  };

  // 标准化键名，与后端 main_key_name 对齐
  const normalizeKey = (key: string): string | null => {
    if (key === " ") return "space";
    if (key === "Enter") return "enter";
    if (key === "Tab") return "tab";
    if (key === "Escape") return "esc";
    if (key === "Delete" || key === "Backspace") return key.toLowerCase();
    if (key === "Control") return "ctrl";
    if (key === "Alt") return "alt";
    if (key === "Shift") return "shift";
    if (key === "Meta") return "meta";
    if (/^[a-z]$/i.test(key)) return key.toLowerCase();
    if (/^[0-9]$/.test(key)) return key;
    if (/^F[0-9]{1,2}$/i.test(key)) return key.toLowerCase();
    return null;
  };

  // 使用实际网格列数，随侧边栏和可用宽度变化。
  const getColumnCount = () => {
    const grid = gridRef.current;
    if (!grid) return 1;
    const columns = window.getComputedStyle(grid).gridTemplateColumns;
    if (!columns || columns === "none") return 1;
    return Math.max(1, columns.trim().split(/\s+/).length);
  };

  // 分类列表：固定的自动分类 + 用户自定义分类
  const builtinCategories = ["游戏", "办公", "开发", "媒体", "社交", "浏览器", "其他"];
  const categories = [
    "全部",
    ...builtinCategories,
    ...(config?.custom_categories ?? []),
  ];
  const categorizedApps = useMemo(() => apps.map((app) => ({
    ...app,
    category: (config?.app_categories?.[app.id] ?? config?.app_categories?.[app.name]) || app.category || "其他",
    hidden_reason: config?.hidden_apps?.includes(app.id) ? "手动隐藏" : app.hidden_reason,
  })), [apps, config]);
  const filtered = useMemo(() => rankApps(categorizedApps.filter((app) =>
    (!app.hidden_reason || (config?.show_all ?? false)) &&
    (activeCategory === "全部" || app.category === activeCategory)
  ), query, config ?? {}), [categorizedApps, activeCategory, query, config]);
  const dockApps = useMemo(() => fixedApps(categorizedApps, config ?? {}), [categorizedApps, config]);
  useEffect(() => {
    const onDockShortcut=(event: KeyboardEvent)=>{
      if(overlayKey || dragging || capturing || event.defaultPrevented)return;
      const index=dockShortcutIndex(event);
      if(index===null)return;
      const target=event.target;
      if(target instanceof HTMLElement && target!==inputRef.current && (target.isContentEditable||target.closest("input,textarea,select")))return;
      event.preventDefault();setAltPressed(false);
      const app=dockApps[index];if(app)void launch(app);
    };
    window.addEventListener("keydown",onDockShortcut);
    return()=>window.removeEventListener("keydown",onDockShortcut);
  },[dockApps,overlayKey,dragging,capturing]);
  const selectedIndex = Math.max(0, filtered.findIndex((app) => app.id === selectedId));
  const setSelectedIndex = (value: number | ((index: number) => number)) => {
    const index = typeof value === "function" ? value(selectedIndex) : value;
    setSelectedId(filtered[index]?.id ?? null);
  };

  const iconTargets = JSON.stringify([...dockApps, ...(searchMode === "apps" ? filtered : [])].filter(app => !app.kind || app.kind === "app").map(({ path, shortcut_path }) => ({ path, shortcut_path })));
  useEffect(() => {
    const loader = iconLoader.current;
    const grid = gridRef.current;
    const dock = dockRef.current;
    if (!loader || !dock) return;
    const targets: IconTarget[] = JSON.parse(iconTargets);
    const byKey = new Map(targets.map((app) => [iconKey(app), app]));
    const visible = new Map<string, number>();
    const dockVisible = new Map<string, number>();
    let disposed = false;
    const publish = () => loader.setVisible([...new Map([...visible, ...dockVisible])].sort((a, b) => a[1] - b[1])
      .map(([key]) => byKey.get(key)).filter((app): app is IconTarget => !!app));
    let observer: IntersectionObserver | undefined;
    if (grid && typeof IntersectionObserver !== "undefined") {
      observer = new IntersectionObserver((entries) => {
        if (disposed) return;
        const bounds = grid.getBoundingClientRect();
        for (const entry of entries) {
          const key = entry.target.getAttribute("data-icon-key");
          if (!key) continue;
          if (entry.isIntersecting) {
            const rect = entry.boundingClientRect;
            visible.set(key, rect.bottom > bounds.top && rect.top < bounds.bottom ? 1 : 2);
          } else {
            visible.delete(key);
          }
        }
        publish();
      }, { root: grid, rootMargin: "120px 0px" });
      grid.querySelectorAll(".app-tile").forEach((tile) => observer!.observe(tile));
    } else if (typeof IntersectionObserver === "undefined") {
      loader.setVisible(targets);
    }
    const dockObserver = typeof IntersectionObserver !== "undefined" ? new IntersectionObserver(entries => {
      if (disposed) return;
      for (const entry of entries) {
        const key = entry.target.getAttribute("data-icon-key");
        if (key) { if (entry.isIntersecting) dockVisible.set(key, 0); else dockVisible.delete(key); }
      }
      publish();
    }, { root: dock, rootMargin: "0px 40px" }) : undefined;
    dock.querySelectorAll('[data-icon-key]').forEach(item => dockObserver?.observe(item));
    if (dockObserver) {
      const bounds = dock.getBoundingClientRect();
      dock.querySelectorAll<HTMLElement>('[data-icon-key]').forEach(item => {
        const rect = item.getBoundingClientRect();
        if (rect.right > bounds.left - 40 && rect.left < bounds.right + 40) dockVisible.set(item.dataset.iconKey!, 0);
      });
      publish();
    }
    const retryTimer = window.setInterval(() => loader.retry(), 30_000);
    return () => {
      disposed = true;
      observer?.disconnect();
      dockObserver?.disconnect();
      window.clearInterval(retryTimer);
      loader.setVisible([]);
    };
  }, [iconTargets, iconRevision, searchMode]);

  // 查询变化时重置选中
  useEffect(() => {
    setSelectedIndex(0);
  }, [query, activeCategory]);

  useEffect(() => {
    if (selectedId && !filtered.some((app) => app.id === selectedId)) setSelectedId(null);
  }, [filtered, selectedId]);

  // 选中项变化时，滚动到可视区域
  useEffect(() => {
    const grid = gridRef.current;
    if (!grid) return;
    const selected = grid.querySelector<HTMLElement>(".app-tile.selected");
    if (selected) {
      selected.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
  }, [selectedIndex]);

  return (
    <div className={`launchpad density-${config?.appearance.density ?? "standard"}${dragging ? ' app-dragging' : ''}`} ref={rootRef} onKeyDown={handleKeyDown} onClickCapture={event => {
      if (appDrag.consumeClick()) { event.preventDefault(); event.stopPropagation(); }
    }} onFocusCapture={event => {
      if (!overlayKey && event.target instanceof HTMLElement) openerRef.current = event.target;
    }}>
      <div className="launcher-content" inert={!!overlayKey}>
      <div className="search-bar" data-tauri-drag-region>
        <button type="button" className="window-drag-handle" aria-label="拖动窗口" title="按住拖动窗口" data-tauri-drag-region />
        <span className="app-wordmark" aria-hidden="true" data-tauri-drag-region>Luma</span>
        <div className="search-mode-switch" role="group" aria-label="搜索模式">
          {([["apps", "应用"], ["files", "文件"]] as const).map(([value, label]) => <button key={value} aria-pressed={searchMode === value} onClick={() => { setSearchMode(value); setQuery(""); setComposing(false); inputRef.current?.focus(); }}>{label}</button>)}
        </div>
        <div className="search-input-wrap"><UiIcon name="search"/>
        <input
          ref={inputRef}
          type="text"
          placeholder={searchMode === "apps" ? "搜索应用、拼音或别名" : "搜索文件名或路径 · 支持 ext:pdf"}
          aria-label={searchMode === "apps" ? "搜索应用" : "搜索文件"}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onCompositionStart={() => setComposing(true)}
          onCompositionEnd={() => setComposing(false)}
          autoFocus
        />
        </div>
        <button className="settings-btn" title="添加快捷入口" aria-label="添加快捷入口" onClick={() => void addManualApp()}><UiIcon name="plus"/></button>
        <button
          className={`settings-btn refresh-btn${refreshing ? " refreshing" : ""}`}
          onClick={() => searchMode === "files" ? fileSearchRef.current?.refresh() : void scan(true)}
          disabled={searchMode === "apps" && refreshing}
          aria-label={searchMode === "files" ? "刷新文件搜索" : refreshing ? "正在扫描应用" : "刷新应用"}
          title={searchMode === "files" ? "刷新文件搜索" : scanError ?? (refreshing ? "正在扫描应用" : "刷新应用")}
        >
          <span aria-hidden="true"><UiIcon name="refresh"/></span>
        </button>
        <button
          className="settings-btn"
          onClick={() => setSettingsOpen(true)}
          title="设置"
          aria-label="设置"
        >
          <UiIcon name="settings"/>
        </button>
      </div>

      <FixedBar apps={dockApps} listRef={dockRef} drag={appDrag.drag} onPlace={placePinned} health={healthById} showHints={altPressed}
        onLaunch={app => void launch(app)} onMenu={(app, x, y) => setMenu({ app, x, y })} onDrag={appDrag.start}/>

      {searchMode === "files" ? <FileSearch ref={fileSearchRef} query={query} composing={composing} suspended={!!overlayKey} onOpened={() => { void invoke("hide_window"); setQuery(""); }}/>
      : <><div className="main-area">
        {/* 分类侧边栏 */}
        <div className="category-sidebar" ref={categoriesRef}>
          {categories.map((cat) => (
            <div
              key={cat}
              className={`category-item${cat === activeCategory ? " active" : ""}${appDrag.drag?.overCategory === (cat === "全部" ? "" : cat) ? " drop-active" : ""}`}
              data-drop-category={cat === "全部" ? "" : cat}
              onClick={() => {
                setActiveCategory(cat);
                setQuery("");
                setTimeout(() => inputRef.current?.focus(), 0);
              }}
              title={cat}
              role="button"
              tabIndex={0}
              aria-pressed={cat === activeCategory}
              onKeyDown={event => {
                if (event.key === "Enter" || event.key === " ") { event.preventDefault(); event.stopPropagation(); setActiveCategory(cat); setQuery(""); }
              }}
            >
              <CategoryIcon category={cat}/>
              <span className="category-name">{cat}</span>
              {cat !== "全部" && !builtinCategories.includes(cat) && (
                <button
                  className="category-remove"
                  onClick={(e) => {
                    e.stopPropagation();
                    removeCustomCategory(cat);
                  }}
                  title={`删除分类 ${cat}`}
                >
                  <UiIcon name="close"/>
                </button>
              )}
            </div>
          ))}
          {/* 添加自定义分类 */}
          <div className="category-add">
            <input
              type="text"
              placeholder="新分类"
              value={categoryDraft}
              onChange={(e) => setCategoryDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  e.stopPropagation();
                  if (categoryDraft.trim()) {
                    addCustomCategory(categoryDraft.trim());
                    setCategoryDraft("");
                  }
                }
              }}
            />
            <button
              className="category-add-btn"
              onClick={() => {
                if (categoryDraft.trim()) {
                  addCustomCategory(categoryDraft.trim());
                  setCategoryDraft("");
                }
              }}
              title="添加分类"
              aria-label="添加分类"
            >
              <UiIcon name="plus"/>
            </button>
          </div>
        </div>

        {/* 应用网格 */}
        <div className="app-grid" ref={gridRef}>
          {loading ? (
            <div className="hint">正在扫描应用...</div>
          ) : filtered.length === 0 ? (
          <div className="hint">{scanError ?? "没有找到匹配的应用"}</div>
          ) : (
            filtered.map((app, idx) => (
              <div
                key={app.id}
                data-icon-key={iconKey(app)}
                className={`app-tile${idx === selectedIndex ? " selected" : ""}`}
                role="button"
                tabIndex={0}
                aria-label={`${app.name}${config?.pinned_apps?.includes(app.id) ? "，已固定" : ""}`}
                onFocus={() => setSelectedIndex(idx)}
                onClick={() => launch(app)}
                onPointerDown={event => appDrag.start(event, app)}
                onContextMenu={(e) => {
                  e.preventDefault();
                  setMenu({ app, x: e.clientX, y: e.clientY });
                }}
                onKeyDown={event => {
                  if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
                    event.preventDefault(); event.stopPropagation();
                    const rect = event.currentTarget.getBoundingClientRect();
                    setMenu({ app, x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 });
                  } else if (event.key === " ") {
                    event.preventDefault(); event.stopPropagation(); void launch(app);
                  }
                }}
                onMouseEnter={() => setSelectedIndex(idx)}
                aria-description={`${app.path}${app.arguments ? `，${app.arguments}` : ""}。单击启动，右键操作菜单，当前分类：${app.category}`}
              >
                {config?.pinned_apps?.includes(app.id) && <span className="app-pin-corner" aria-hidden="true"><UiIcon name="star"/></span>}
                {healthById.has(app.id) && <button className={`app-health-badge health-${healthById.get(app.id)!.state}`} title={healthById.get(app.id)!.message} aria-label={`修复 ${app.name}：${healthById.get(app.id)!.message}`} onPointerDown={event=>event.stopPropagation()} onKeyDown={event=>event.stopPropagation()} onClick={event=>{event.stopPropagation();repairEntry(app);}}><UiIcon name="info"/></button>}
                <div className="app-icon">
                  <EntryIcon app={app}/>
                </div>
                <span className="app-name">{app.name}</span>
              </div>
            ))
          )}
        </div>
      </div>

      <div className="footer-hint">
        <span className="footer-status">{filtered.length} 个应用<span className="footer-shortcuts"> · ↑↓←→ 导航 · Enter 启动 · Esc 收起</span></span>
        <button className="footer-link" onClick={() => openDiagnostics(null)}>应用诊断</button>
        <button className="footer-link" onClick={openHealth}>入口检查{healthById.size ? ` · ${healthById.size}` : ""}</button>
        <label className="show-all-toggle" title="勾选后显示被自动过滤的卸载程序、命令行工具等">
          <input
            type="checkbox"
            checked={config?.show_all ?? false}
            onChange={async (e) => {
              const showAll = e.target.checked;
              try {
                await invoke("set_show_all", { showAll });
                setConfig((c) => (c ? { ...c, show_all: showAll } : c));
              } catch (err) {
                console.error("设置显示全部失败", err);
              } finally {
                // 点击复选框后把焦点还给搜索框，保证后续键入与视觉焦点一致
                setTimeout(() => inputRef.current?.focus(), 0);
              }
            }}
          />
          <span>显示全部</span>
        </label>
      </div>

      </>}
      </div>
      {appDrag.drag && <div className="app-drag-ghost" aria-hidden="true" style={{ left: Math.min(appDrag.drag.x + 14, window.innerWidth - 210), top: Math.min(appDrag.drag.y + 14, window.innerHeight - 64) }}>
        <EntryIcon app={appDrag.drag.app}/>
        <span>{appDrag.drag.app.name}<small>{appDrag.drag.overCategory !== null ? (appDrag.drag.overCategory ? `松开移到「${appDrag.drag.overCategory}」` : "松开恢复自动分类") : appDrag.drag.overDock ? "松开固定到此处" : "拖到分类或上方固定栏"}</small></span>
      </div>}
      {/* 设置面板 */}
      {settingsOpen && config && (
        <Dialog title="设置" label="关闭设置" onClose={hideLauncher}>

            <div className="settings-section">
              <div className="settings-label">Luma {snapshot?.version ?? "1.2.0"}</div>
              <div className="runtime-path" title={snapshot?.executable}>{snapshot?.executable ?? "正在读取运行信息…"}</div>
              <div className="panel-actions">
                <button className="btn" onClick={() => openDiagnostics(null)}>应用诊断</button>
                <button className="btn" onClick={() => void invoke("restart_app")}>重新启动</button>
                <button className="btn" onClick={() => void invoke("quit_app")}>退出 Luma</button>
              </div>
            </div>

            <ManagementTools onImported={personalSaved} onAdd={() => void addManualApp()} onPreview={setPreview}/>

            {/* 唤醒快捷键 */}
            <div className="settings-section">
              <div className="settings-label">唤醒快捷键</div>
              <div className="settings-row">
                <div className="hotkey-display">
                  {capturing
                    ? "请按下新快捷键..."
                    : hotkeyLabel(hotkeyDraft ?? config.hotkey)}
                </div>
                {capturing ? (
                  <button
                    className="btn"
                    onClick={() => setCapturing(false)}
                  >
                    取消
                  </button>
                ) : (
                  <button
                    className="btn"
                    onClick={() => {
                      setCapturing(true);
                      setHotkeyDraft(null);
                    }}
                  >
                    重新设置
                  </button>
                )}
              </div>
              {hotkeyDraft && !capturing && (
                <div className="settings-hint-row">
                  <span className="settings-hint">
                    已捕获: {hotkeyLabel(hotkeyDraft)}
                  </span>
                  <button
                    className="btn btn-primary"
                    onClick={() => {
                      if (config && hotkeyDraft) {
                        saveConfig({ ...config, hotkey: hotkeyDraft });
                        setHotkeyDraft(null);
                      }
                    }}
                  >
                    应用
                  </button>
                </div>
              )}
              {capturing && (
                <div className="settings-hint">
                  支持组合键（如 Ctrl+Space、Alt+Space）。提示：Win 键组合可能被系统占用。
                </div>
              )}
            </div>

            {/* 开机自启 */}
            <div className="settings-section">
              <div className="settings-label">开机自启</div>
              <div className="settings-row">
                <span className="settings-hint">登录时自动启动 Luma</span>
                <label className="switch">
                  <input
                    type="checkbox"
                    aria-label="登录时自动启动 Luma"
                    disabled={autostartBusy}
                    checked={config.autostart}
                    onChange={async (e) => {
                      const enabled = e.target.checked;
                      try {
                        const status = await invoke<AutostartStatus>("set_autostart", { enabled });
                        setAutostart(status);
                        setConfig(current => current ? { ...current, autostart: status.enabled } : current);
                      } catch (err) {
                        console.error("设置自启失败", err);
                        setNotice(`设置自启失败: ${err}`);
                      }
                    }}
                  />
                  <span className="switch-slider"></span>
                </label>
              </div>
              {autostart?.legacy && <div className="settings-hint">检测到旧版计划任务。迁移需在 Windows 管理员提示中确认，随后改用当前用户自启项。<button className="btn btn-sm" disabled={autostartBusy} onClick={async () => {
                setAutostartBusy(true);
                try { const status = await invoke<AutostartStatus>("migrate_legacy_autostart"); setAutostart(status); setConfig(current => current ? { ...current, autostart: status.enabled } : current); setNotice("已迁移到当前用户登录自启"); }
                catch (error) { setNotice(String(error)); }
                finally { setAutostartBusy(false); }
              }}>{autostartBusy ? "等待 Windows 管理员确认…" : "管理员迁移旧版自启"}</button></div>}
            </div>

            {/* 窗口外观 */}
            <div className="settings-section">
              <div className="settings-label">窗口外观</div>
              <div className="window-layout-controls">
                <div className="density-choices" role="group" aria-label="卡片显示密度">{([['compact','紧凑'],['standard','标准'],['comfortable','宽松']] as const).map(([density,label])=><button key={density} className="btn" aria-pressed={(config.appearance.density??'standard')===density} onClick={()=>{changeAppearance({density});flushAppearance();}}>{label}</button>)}</div>
                <div className="window-size-row">
                  <span className="settings-hint">拖动窗口边缘调整大小，尺寸自动保存。</span>
                  <button className="btn" onClick={()=>void invoke("reset_window_size").catch(error=>setNotice(String(error)))}>恢复默认尺寸</button>
                </div>
              </div>

              <div className="settings-row">
                <span className="settings-hint">玻璃深度</span>
                <input
                  type="range"
                  min="0.3"
                  max="0.95"
                  step="0.05"
                  value={config.appearance.opacity}
                  aria-label="玻璃深度"
                  onChange={event => changeAppearance({ opacity: Number(event.target.value) })}
                  onPointerUp={flushAppearance}
                  onKeyUp={flushAppearance}
                  onBlur={flushAppearance}
                />
                <span className="settings-value">
                  {Math.round(config.appearance.opacity * 100)}%
                </span>
              </div>

              <div className="settings-row">
                <span className="settings-hint">圆角大小</span>
                <input
                  type="range"
                  min="0"
                  max="24"
                  step="2"
                  value={config.appearance.corner_radius}
                  aria-label="圆角大小"
                  onChange={event => changeAppearance({ corner_radius: Number(event.target.value) })}
                  onPointerUp={flushAppearance}
                  onKeyUp={flushAppearance}
                  onBlur={flushAppearance}
                />
                <span className="settings-value">
                  {config.appearance.corner_radius}px
                </span>
              </div>
            </div>

            {/* 额外扫描目录 */}
            <div className="settings-section">
              <div className="settings-label">额外扫描目录</div>
              <div className="settings-hint">
                绿色版游戏等未注册到开始菜单的程序目录（如 D:\game），将自动扫描其中的 exe
              </div>
              <div className="extra-dirs-list">
                {(config.extra_dirs ?? []).map((dir, i) => (
                  <div className="extra-dir-item" key={i}>
                    <span className="extra-dir-path" title={dir}>{dir}</span>
                    <button
                      className="btn btn-sm"
                      onClick={() =>
                        saveExtraDirs((config.extra_dirs ?? []).filter((_, j) => j !== i))
                      }
                    >
                      移除
                    </button>
                  </div>
                ))}
              </div>
              <div className="settings-row">
                <input
                  type="text"
                  className="extra-dir-input"
                  placeholder="输入目录路径，如 D:\game"
                  value={extraDirDraft}
                  onChange={(e) => setExtraDirDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && extraDirDraft.trim()) {
                      saveExtraDirs([...(config.extra_dirs ?? []), extraDirDraft.trim()]);
                      setExtraDirDraft("");
                    }
                  }}
                />
                <button
                  className="btn btn-primary"
                  onClick={() => {
                    if (extraDirDraft.trim()) {
                      saveExtraDirs([...(config.extra_dirs ?? []), extraDirDraft.trim()]);
                      setExtraDirDraft("");
                    }
                  }}
                >
                  添加
                </button>
              </div>
            </div>

            {/* 应用分类管理 */}
            <div className="settings-section">
              <div className="settings-label">应用分类</div>
              <div className="settings-hint">
                为应用手动指定分类（覆盖自动分类）。自动分类：游戏/办公/开发/媒体/社交/浏览器/其他
              </div>
              <div className="category-app-list">
                {categorizedApps
                  .filter((a) => !a.hidden_reason || config.show_all)
                  .map((app) => {
                    const current = config.app_categories?.[app.id] ?? config.app_categories?.[app.name] ?? "";
                    return (
                      <div className="category-app-item" key={app.id}>
                        <span className="category-app-name" title={`${app.path}\n${app.arguments}`}>
                          {app.name}
                        </span>
                        <select
                          className="category-app-select"
                          value={current}
                          onChange={(e) => setAppCategory(app.id, e.target.value)}
                        >
                          <option value="">自动分类</option>
                          {[...builtinCategories, ...(config.custom_categories ?? [])].map(
                            (cat) => (
                              <option key={cat} value={cat}>
                                {cat}
                              </option>
                            )
                          )}
                        </select>
                      </div>
                    );
                  })}
              </div>
            </div>
        </Dialog>
      )}
      {diagnosticsOpen && <DiagnosticsPanel key={diagnosticId ?? "all"} apps={categorizedApps} config={config}
        snapshot={snapshot} initialId={diagnosticId} onClose={() => setDiagnosticsOpen(false)}
        onRefresh={() => void refreshDiagnostics()} onPin={(app) => void togglePinned(app)} onHide={(app) => void toggleHidden(app)}/>}
      {healthOpen && <HealthPanel apps={apps} checks={healthChecks} busy={healthBusy} error={healthError} onClose={()=>setHealthOpen(false)} onCheck={()=>void recheckHealth()} onRepair={repairEntry}/>}
      {editor && config && <AppEditor key={"app" in editor ? editor.app.id : "repair" in editor ? `repair:${editor.repair.id}` : editor.manual.id} target={editor} config={config} onClose={() => setEditor(null)} onSaved={personalSaved}/>}
      {preview && <ImportPreview preview={preview} onClose={() => setPreview(null)} onImported={personalSaved}/>}
      {menu && <AppMenu app={menu.app} config={config} x={menu.x} y={menu.y} onClose={() => setMenu(null)}
        onEdit={() => { setEditor({ app: menu.app }); setMenu(null); }} onDetails={() => { openDiagnostics(menu.app.id); setMenu(null); }}
        onRepair={()=>repairEntry(menu.app)}
        onLaunch={(admin, bypass) => { void launch(menu.app, admin, bypass); setMenu(null); }} onPin={() => { void togglePinned(menu.app); setMenu(null); }}
        onHide={() => { void toggleHidden(menu.app); setMenu(null); }} onMessage={setNotice}/>}
      {runningChoice && <RunningAppDialog key={runningChoice.app.id} app={runningChoice.app} initial={runningChoice.info}
        onClose={hideLauncher} onActivate={id => activateWindow(runningChoice.app, id)}
        onLaunch={newWindow => launch(runningChoice.app, false, true, newWindow)}/>}
      {notice && !editor && !preview && !menu && !healthOpen && !runningChoice && <div className="toast" role="status"><span>{notice}</span><button aria-label="关闭提示" onClick={() => setNotice("")}><UiIcon name="close"/></button></div>}
      <span className="resize-grip" aria-hidden="true"/>
    </div>
  );
}

export default App;
