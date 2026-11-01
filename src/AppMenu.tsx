import { invoke } from "@tauri-apps/api/core";
import type { AppEntry, Config } from "./types";
interface Props { app: AppEntry; config: Config | null; x: number; y: number; onClose: () => void; onEdit: () => void; onDetails: () => void; onLaunch: (admin: boolean) => void; onPin: () => void; onHide: () => void; onMessage: (message: string) => void }
export function AppMenu({ app, config, x, y, onClose, onEdit, onDetails, onLaunch, onPin, onHide, onMessage }: Props) {
  const action = async (command: string) => { try { await invoke(command, { appId: app.id }); onMessage(command === "copy_app_path" ? "已复制应用路径" : "已打开所在目录"); } catch (error) { onMessage(String(error)); } onClose(); };
  const managed = /^shell:|^::\{|:\/\//i.test(app.path);
  return <div className="menu-layer" onClick={onClose} onContextMenu={event => { event.preventDefault(); onClose(); }}>
    <section className="app-menu" role="menu" aria-label={`${app.name} 操作`} style={{ left: Math.max(8, Math.min(x, window.innerWidth - 242)), top: Math.max(8, Math.min(y, window.innerHeight - 384)) }} onClick={event => event.stopPropagation()}>
      <strong>{app.name}</strong><button role="menuitem" onClick={() => onLaunch(false)}>启动应用</button>
      <button role="menuitem" disabled={managed} onClick={() => onLaunch(true)}>以管理员身份启动</button>
      <button role="menuitem" onClick={onEdit}>别名与启动设置</button>
      <button role="menuitem" onClick={() => void action("reveal_app")}>打开所在目录</button>
      <button role="menuitem" onClick={() => void action("copy_app_path")}>复制应用路径</button>
      <button role="menuitem" onClick={onPin}>{config?.pinned_apps?.includes(app.id) ? "取消置顶" : "置顶应用"}</button>
      <button role="menuitem" onClick={onHide}>{config?.hidden_apps?.includes(app.id) ? "取消手动隐藏" : "手动隐藏"}</button>
      <button role="menuitem" onClick={onDetails}>查看应用详情</button>
    </section>
  </div>;
}
