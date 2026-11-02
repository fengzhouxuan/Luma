import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Dialog } from "./Dialog";
import { EntryIcon } from "./EntryIcon";
import type { AppEntry } from "./types";
import "./runningApps.css";

export interface RunningWindows {
  windows: { id: string; title: string; minimized: boolean }[];
  can_new_window: boolean;
}

export function RunningAppDialog({ app, initial, onClose, onActivate, onLaunch }: {
  app: AppEntry; initial: RunningWindows; onClose: () => void;
  onActivate: (id: string) => Promise<void>; onLaunch: (newWindow: boolean) => Promise<void>;
}) {
  const [info, setInfo] = useState(initial);
  const [refreshing, setRefreshing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const alive = useRef(true);
  const pending = useRef(false);
  useEffect(() => { alive.current = true; return () => { alive.current = false; }; }, []);
  const act = async (job: () => Promise<void>) => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError("");
    try { await job(); }
    catch (error) { if (alive.current) setError(String(error)); }
    finally { pending.current = false; if (alive.current) setBusy(false); }
  };
  const refresh = async () => {
    if (pending.current || refreshing) return;
    setRefreshing(true); setError("");
    try {
      const next = await invoke<RunningWindows>("get_running_windows", { appId: app.id });
      if (alive.current) setInfo(next);
    } catch (error) { if (alive.current) setError(String(error)); }
    finally { if (alive.current) setRefreshing(false); }
  };
  return <Dialog title="应用已在运行" label="取消窗口选择" className="running-panel" busy={busy} onClose={onClose}>
    <div className="running-app"><EntryIcon app={app}/><strong>{app.name}</strong></div>
    <p className="settings-hint">选择一个窗口切换过去，或{info.can_new_window ? "打开新窗口" : "再次启动应用"}。</p>
    {app.arguments.trim() && <p className="settings-hint">以下是同一程序的窗口，可能属于不同配置；切换窗口不会重新应用此入口的启动参数。</p>}
    <div className="running-windows" aria-label="现有窗口" aria-busy={refreshing}>
      {info.windows.map((window, index) => <button key={window.id} className="running-window" disabled={busy || refreshing}
        data-initial-focus={index === 0 ? true : undefined} onClick={() => void act(() => onActivate(window.id))}>
        <span className="running-window-title" title={window.title}>{window.title}</span>
        <span className="running-window-action">{window.minimized ? "恢复并切换" : "切换窗口"}</span>
      </button>)}
      {!info.windows.length && <p className="settings-hint" role="status">当前没有可切换的窗口，可以再次启动。</p>}
    </div>
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="running-actions">
      <button className="btn" disabled={busy || refreshing} onClick={() => void refresh()}>{refreshing ? "刷新中…" : "刷新窗口"}</button>
      <button className="btn primary" disabled={busy || refreshing} onClick={() => void act(() => onLaunch(info.can_new_window))}>{info.can_new_window ? "新开窗口" : "再次启动应用"}</button>
    </div>
    {!info.can_new_window && <p className="settings-hint">再次启动会保留原入口的参数；单实例应用可能仍会使用已有窗口。</p>}
  </Dialog>;
}
