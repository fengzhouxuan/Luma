import { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { pinyin } from "pinyin-pro";
import "./App.css";

interface AppEntry {
  name: string;
  path: string;
  icon: string | null;
}

// 与后端 config.rs 对应的配置类型
interface HotkeyDouble {
  mode: "double";
  key: string; // alt / ctrl / shift
}
interface HotkeyCombo {
  mode: "combo";
  modifiers: string[]; // ["ctrl","alt","shift"]
  key: string; // 主键，如 "space" / "a" / "f1"
}
type HotkeyConfig = HotkeyDouble | HotkeyCombo;

interface Config {
  hotkey: HotkeyConfig;
  autostart: boolean;
  appearance: {
    opacity: number;
    corner_radius: number;
  };
}

// 快捷键的可读显示名
function hotkeyLabel(h: HotkeyConfig): string {
  if (h.mode === "double") {
    const names: Record<string, string> = { alt: "Alt", ctrl: "Ctrl", shift: "Shift" };
    return `双击 ${names[h.key] ?? h.key}`;
  }
  const modNames: Record<string, string> = { ctrl: "Ctrl", alt: "Alt", shift: "Shift", meta: "Win" };
  const mods = h.modifiers.map((m) => modNames[m] ?? m).join(" + ");
  const keyName = h.key.length === 1 ? h.key.toUpperCase() : h.key;
  return `${mods}${mods ? " + " : ""}${keyName}`;
}

function App() {
  const [apps, setApps] = useState<AppEntry[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [config, setConfig] = useState<Config | null>(null);
  const [capturing, setCapturing] = useState(false);
  const [hotkeyDraft, setHotkeyDraft] = useState<HotkeyConfig | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const appWindow = getCurrentWindow();

  // 扫描应用（首次加载时执行一次）
  const scan = useCallback(async () => {
    try {
      const result = await invoke<AppEntry[]>("scan_apps");
      setApps(result);
      loadIcons(result);
    } catch (e) {
      console.error("扫描失败", e);
    } finally {
      setLoading(false);
    }
  }, []);

  // 逐个异步加载图标
  const loadIcons = useCallback(async (list: AppEntry[]) => {
    for (const app of list) {
      try {
        const icon = await invoke<string | null>("get_icon", { path: app.path });
        if (icon) {
          setApps((prev) =>
            prev.map((a) => (a.path === app.path ? { ...a, icon } : a))
          );
        }
      } catch {
        // 单个图标加载失败忽略
      }
    }
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

  useEffect(() => {
    scan();
    loadConfig();
    setTimeout(() => inputRef.current?.focus(), 100);
  }, [scan, loadConfig]);

  // 应用外观配置到 CSS 变量
  useEffect(() => {
    if (!config) return;
    const root = document.documentElement;
    root.style.setProperty("--mv-launchpad-opacity", String(config.appearance.opacity));
    root.style.setProperty("--mv-launchpad-radius", `${config.appearance.corner_radius}px`);
  }, [config]);

  // 保存配置
  const saveConfig = async (cfg: Config) => {
    try {
      await invoke("set_config", { cfg });
      setConfig(cfg);
    } catch (e) {
      console.error("保存配置失败", e);
      alert(`保存配置失败: ${e}`);
    }
  };

  // 启动应用
  const launch = async (app: AppEntry, admin: boolean = false) => {
    try {
      await invoke("launch_app", { path: app.path, admin });
      await appWindow.hide();
      setQuery("");
    } catch (e) {
      console.error("启动失败", e);
      alert(`启动失败: ${e}`);
    }
  };

  // 关闭启动器（含设置面板）
  const hideLauncher = () => {
    if (settingsOpen) {
      setSettingsOpen(false);
      setCapturing(false);
      // 关键：关闭面板后把焦点还给搜索框。
      // 否则焦点会落在已卸载的按钮上、回退到 body，
      // 导致后续键盘事件不再冒泡到 .launchpad，Esc 失效。
      setTimeout(() => inputRef.current?.focus(), 0);
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
      e.preventDefault();
      hideLauncher();
      return;
    }

    // 设置面板打开时，不处理网格导航
    if (settingsOpen) return;

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
        launch(app, e.ctrlKey || e.metaKey);
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

    if (mods.length === 0) {
      // 无修饰键：只能是"双击单键"模式，仅支持 alt/ctrl/shift
      if (key === "alt" || key === "ctrl" || key === "shift") {
        return { mode: "double", key };
      }
      // 其他单键（如字母）无修饰键不支持，忽略
      return null;
    }

    // 有修饰键：组合键模式，但主键不能是修饰键
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

  // 根据窗口宽度估算网格列数（用于方向键导航）
  const getColumnCount = () => {
    const width = window.innerWidth || 900;
    return Math.max(1, Math.floor((width - 64) / 112));
  };

  // 拼音搜索：匹配名称、全拼、首字母
  const filtered = apps.filter((app) => {
    if (!query.trim()) return true;
    const q = query.toLowerCase().trim();
    const name = app.name.toLowerCase();
    if (name.includes(q)) return true;

    try {
      const fullPinyin = pinyin(app.name, { toneType: "none", type: "array" })
        .join("")
        .toLowerCase();
      const firstLetters = pinyin(app.name, {
        pattern: "first",
        toneType: "none",
        type: "array",
      })
        .join("")
        .toLowerCase();

      if (fullPinyin.includes(q) || firstLetters.includes(q)) return true;
    } catch {
      // 拼音转换失败则忽略
    }
    return false;
  });

  // 查询变化时重置选中
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

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
    <div className="launchpad" onKeyDown={handleKeyDown}>
      <div className="search-bar">
        <input
          ref={inputRef}
          type="text"
          placeholder="搜索应用（支持拼音）..."
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
        />
        <button
          className="settings-btn"
          onClick={() => setSettingsOpen(true)}
          title="设置"
        >
          ⚙
        </button>
      </div>

      <div className="app-grid" ref={gridRef}>
        {loading ? (
          <div className="hint">正在扫描应用...</div>
        ) : filtered.length === 0 ? (
          <div className="hint">没有找到匹配的应用</div>
        ) : (
          filtered.map((app, idx) => (
            <div
              key={app.path + idx}
              className={`app-tile${idx === selectedIndex ? " selected" : ""}`}
              onClick={() => launch(app)}
              onContextMenu={(e) => {
                e.preventDefault();
                launch(app, true);
              }}
              onMouseEnter={() => setSelectedIndex(idx)}
              title={`${app.name}\n单击启动 · 右键以管理员启动`}
            >
              <div className="app-icon">
                {app.icon ? (
                  <img src={app.icon} alt={app.name} />
                ) : (
                  <div className="app-icon-fallback">
                    {app.name.charAt(0).toUpperCase()}
                  </div>
                )}
              </div>
              <span className="app-name">{app.name}</span>
            </div>
          ))
        )}
      </div>

      <div className="footer-hint">
        单击启动 · 右键管理员 · ↑↓←→ 导航 · Ctrl+回车 管理员 · Esc 隐藏
      </div>

      {/* 设置面板 */}
      {settingsOpen && config && (
        <div className="settings-overlay" onClick={() => hideLauncher()}>
          <div
            className="settings-panel"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="settings-header">
              <span>设置</span>
              <button className="settings-close" onClick={() => hideLauncher()}>
                ×
              </button>
            </div>

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
                  支持：双击单键（Alt/Ctrl/Shift）或组合键（如 Ctrl+Space）。提示：Alt+Space 被系统占用。
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
                    checked={config.autostart}
                    onChange={async (e) => {
                      const enabled = e.target.checked;
                      try {
                        await invoke("set_autostart", { enabled });
                        setConfig({ ...config, autostart: enabled });
                      } catch (err) {
                        console.error("设置自启失败", err);
                        alert(`设置自启失败: ${err}`);
                      }
                    }}
                  />
                  <span className="switch-slider"></span>
                </label>
              </div>
            </div>

            {/* 窗口外观 */}
            <div className="settings-section">
              <div className="settings-label">窗口外观</div>

              <div className="settings-row">
                <span className="settings-hint">背景透明度</span>
                <input
                  type="range"
                  min="0.3"
                  max="0.95"
                  step="0.05"
                  value={config.appearance.opacity}
                  onChange={(e) => {
                    const opacity = Number(e.target.value);
                    setConfig({ ...config, appearance: { ...config.appearance, opacity } });
                  }}
                  onMouseUp={() => saveConfig(config)}
                  onTouchEnd={() => saveConfig(config)}
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
                  onChange={(e) => {
                    const corner_radius = Number(e.target.value);
                    setConfig({ ...config, appearance: { ...config.appearance, corner_radius } });
                  }}
                  onMouseUp={() => saveConfig(config)}
                  onTouchEnd={() => saveConfig(config)}
                />
                <span className="settings-value">
                  {config.appearance.corner_radius}px
                </span>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default App;
