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

function App() {
  const [apps, setApps] = useState<AppEntry[]>([]);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const gridRef = useRef<HTMLDivElement>(null);
  const appWindow = getCurrentWindow();

  // 扫描应用（首次加载时执行一次）
  const scan = useCallback(async () => {
    try {
      const result = await invoke<AppEntry[]>("scan_apps");
      setApps(result);
      // 异步加载图标（不阻塞 UI，逐个填充）
      loadIcons(result);
    } catch (e) {
      console.error("扫描失败", e);
    } finally {
      setLoading(false);
    }
  }, []);

  // 逐个异步加载图标
  const loadIcons = useCallback(async (list: AppEntry[]) => {
    // 只加载可见区域内的图标，避免一次性请求过多；这里简化为并发逐个加载
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

  useEffect(() => {
    scan();
    setTimeout(() => inputRef.current?.focus(), 100);
  }, [scan]);

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

  // 键盘导航：方向键 + 回车启动 + Ctrl+回车管理员 + Esc 隐藏
  // 关键：Esc 必须在前端处理！因为焦点在本窗口（WebView2/Chromium 内核）时，
  // Windows 会静默停止向后端 WH_KEYBOARD_LL 钩子投递键盘事件（Chromium 焦点下的已知问题），
  // 所以后端钩子收不到 Esc，只能靠前端 keydown 兜底关闭。
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      // 走后端 hide_window，同步复位后端权威显示标志
      invoke("hide_window");
      setQuery("");
      return;
    }

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

    // 拼音匹配
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
    </div>
  );
}

export default App;
