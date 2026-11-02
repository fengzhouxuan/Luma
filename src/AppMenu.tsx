import { invoke } from "@tauri-apps/api/core";
import { useLayoutEffect, useRef } from "react";
import { focusableElements } from "./overlayFocus";
import type { AppEntry, Config } from "./types";
interface Props { app: AppEntry; config: Config | null; x: number; y: number; onClose: () => void; onEdit: () => void; onRepair: () => void; onDetails: () => void; onLaunch: (admin: boolean, bypass?: boolean) => void; onPin: () => void; onHide: () => void; onMessage: (message: string) => void }
export function AppMenu({ app, config, x, y, onClose, onEdit, onRepair, onDetails, onLaunch, onPin, onHide, onMessage }: Props) {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const menu = ref.current;
    if (!menu) return;
    const rect = menu.getBoundingClientRect();
    menu.style.left = `${Math.max(8, Math.min(x, window.innerWidth - rect.width - 8))}px`;
    menu.style.top = `${Math.max(8, Math.min(y, window.innerHeight - rect.height - 8))}px`;
  }, [x, y]);
  const action = async (command: string) => { try { await invoke(command, { appId: app.id }); onMessage(command === "copy_app_path" ? "已复制应用路径" : "已打开所在目录"); } catch (error) { onMessage(String(error)); } onClose(); };
  const managed = app.kind === "folder" || /^shell:|^::\{|:\/\//i.test(app.path);
  return <div className="menu-layer" onClick={onClose} onContextMenu={event => { event.preventDefault(); onClose(); }}>
    <section ref={ref} data-focus-scope tabIndex={-1} className="app-menu" role="menu" aria-label={`${app.name} 操作`} style={{ left: x, top: y }} onClick={event => event.stopPropagation()}
      onKeyDown={event => {
        if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
        event.preventDefault(); event.stopPropagation();
        const items = focusableElements(event.currentTarget);
        const current = items.indexOf(document.activeElement as HTMLElement);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 :
          (current + (event.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
        items[next]?.focus();
      }}>
      <strong>{app.name}</strong><button role="menuitem" onClick={() => onLaunch(false)}>启动应用</button>
      {(!app.kind || app.kind === "app") && <button role="menuitem" onClick={() => onLaunch(false, true)}>再次启动（跳过窗口选择）</button>}
      <button role="menuitem" disabled={managed} onClick={() => onLaunch(true)}>以管理员身份启动</button>
      <button role="menuitem" onClick={onEdit}>别名与启动设置</button>
      <button role="menuitem" onClick={onRepair}>重新选择入口目标</button>
      <button role="menuitem" disabled={app.kind === "web"} onClick={() => void action("reveal_app")}>打开所在目录</button>
      <button role="menuitem" onClick={() => void action("copy_app_path")}>{app.kind === "web" ? "复制链接" : "复制应用路径"}</button>
      <button role="menuitem" onClick={onPin}>{config?.pinned_apps?.includes(app.id) ? "从应用栏取消固定" : "固定到应用栏"}</button>
      <button role="menuitem" onClick={onHide}>{config?.hidden_apps?.includes(app.id) ? "取消手动隐藏" : "手动隐藏"}</button>
      <button role="menuitem" onClick={onDetails}>查看应用详情</button>
    </section>
  </div>;
}
