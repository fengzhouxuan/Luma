import { useState } from "react";
import { Dialog } from "./Dialog";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { BackupPreview, Config, UpdateInfo } from "./types";
interface Props { onImported: (config: Config) => void; onAdd: () => void; onPreview: (preview: BackupPreview) => void }
export function ManagementTools({ onAdd, onPreview }: Props) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const run = async (action: () => Promise<void>) => { setBusy(true); setMessage(""); try { await action(); } catch (error) { setMessage(String(error)); } finally { setBusy(false); } };
  return <>
    <div className="settings-section"><div className="settings-label">应用与配置</div>
      <div className="panel-actions"><button className="btn" disabled={busy} onClick={onAdd}>添加快捷入口</button>
        <button className="btn" disabled={busy} onClick={() => void run(async () => { const path = await invoke<string | null>("export_config_backup"); if (path) setMessage(`已导出：${path}`); })}>导出配置</button>
        <button className="btn" disabled={busy} onClick={() => void run(async () => { const preview = await invoke<BackupPreview | null>("preview_config_import"); if (preview) onPreview(preview); })}>导入配置</button></div>
      <p className="settings-hint">支持应用、文件夹、网页和脚本；本机目标也可直接拖入。备份包括分类、固定栏、隐藏、别名、快捷入口及扫描目录；保留本机快捷键、自启和窗口位置。</p>
    </div>
    <div className="settings-section"><div className="settings-label">应用更新</div><div className="panel-actions">
      <button className="btn" disabled={busy} onClick={() => void run(async () => setUpdate(await invoke<UpdateInfo>("check_update")))}>{busy ? "处理中…" : "检查更新"}</button>
      <button className="btn" onClick={() => void openUrl("https://github.com/fengzhouxuan/Luma/releases")}>官方版本页面</button></div>
      {update && <div className="update-result"><strong>当前 {update.current} · 已发布 {update.version}</strong><p>{update.message}</p>
        {update.notes && <details><summary>更新日志</summary><pre>{update.notes}</pre></details>}
        {update.available && update.can_install && <button className="btn btn-primary" disabled={busy} onClick={() => void run(async () => { setMessage("正在下载并校验安装包，通过后将退出并升级…"); await invoke("install_update", { version: update.version }); })}>下载并安装更新</button>}</div>}
    </div>
    {message && <p className="operation-message" role="status">{message}</p>}
  </>;
}

export function ImportPreview({ preview, onClose, onImported }: { preview: BackupPreview; onClose: () => void; onImported: (config: Config) => void }) {
  const [error, setError] = useState(""); const [busy, setBusy] = useState(false);
  return <Dialog title="确认导入配置" label="取消导入" className="editor-panel" busy={busy} onClose={onClose}>
    <p>快捷入口 {preview.manual_apps} · 搜索别名 {preview.aliases} · 分类 {preview.categories}</p><p>固定应用 {preview.pinned} · 隐藏 {preview.hidden} · 扫描目录 {preview.directories}</p>
    <p className="settings-hint">导入将替换上述个人配置。当前配置会先自动备份；本机快捷键、自启和显示器位置保留。</p>
    {preview.missing.length > 0 && <details open><summary>{preview.missing.length} 个路径在本机不存在，导入后可修正</summary><pre className="path-list">{preview.missing.join("\n")}</pre></details>}
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="panel-actions"><button className="btn btn-primary" disabled={busy} onClick={async () => { setBusy(true); try { onImported(await invoke<Config>("import_config_backup", { json: preview.json })); } catch (error) { setError(String(error)); } finally { setBusy(false); } }}>确认导入</button><button className="btn" disabled={busy} onClick={onClose}>取消</button></div>
  </Dialog>;
}
