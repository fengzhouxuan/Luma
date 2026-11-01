import { useState } from "react";
import { Dialog } from "./Dialog";
import { invoke } from "@tauri-apps/api/core";
import type { AppEntry, Config, ManualApp, LaunchOverride } from "./types";

export type EditorTarget = { app: AppEntry } | { manual: ManualApp };
interface Props { target: EditorTarget; config: Config; onClose: () => void; onSaved: (config: Config) => void }
export function AppEditor({ target, config, onClose, onSaved }: Props) {
  const app = "app" in target ? target.app : null;
  const initial = "manual" in target ? target.manual : config.manual_apps?.find(item => item.id === app?.id);
  const [manual, setManual] = useState<ManualApp | null>(initial ?? null);
  const id = manual?.id ?? app!.id;
  const [aliases, setAliases] = useState((config.aliases?.[id] ?? []).join("\n"));
  const [custom, setCustom] = useState(!!config.launch_overrides?.[id]);
  const [args, setArgs] = useState(config.launch_overrides?.[id]?.arguments ?? manual?.arguments ?? app?.arguments ?? "");
  const [directory, setDirectory] = useState(config.launch_overrides?.[id]?.working_dir ?? manual?.working_dir ?? app?.working_dir ?? "");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [removeConfirm, setRemoveConfirm] = useState(false);
  const shellManaged = !manual && !!app && (/^shell:|^::\{|:\/\//i.test(app.path));
  const run = async (task: () => Promise<void>) => { setBusy(true); setError(""); try { await task(); } catch (error) { setError(String(error)); } finally { setBusy(false); } };
  const browse = () => void run(async () => {
    const next = await invoke<ManualApp | null>("pick_manual_app");
    if (next) { setManual({ ...next, id }); setArgs(next.arguments); setDirectory(next.working_dir ?? ""); }
  });
  const save = () => void run(async () => {
    const values = aliases.split(/[\n,，]+/);
    let next: Config;
    if (manual) {
      next = await invoke<Config>("save_manual_app", { item: { ...manual, name: manual.name.trim(), arguments: args, working_dir: directory.trim() || null }, aliases: values });
    } else {
      const launch: LaunchOverride | null = custom && !shellManaged ? { arguments: args, working_dir: directory.trim() || null } : null;
      next = await invoke<Config>("save_app_options", { appId: id, aliases: values, launch });
    }
    onSaved(next);
  });
  return <Dialog title={manual ? "手动应用" : `应用设置 · ${app?.name}`} label="关闭应用设置" className="editor-panel" busy={busy} onClose={onClose}>
      {manual ? <><label className="form-field">应用名称<input aria-label="应用名称" value={manual.name} onChange={event => setManual({ ...manual, name: event.target.value })}/></label>
        <label className="form-field">程序路径<div className="settings-row"><input aria-label="程序路径" readOnly value={manual.path}/><button className="btn" disabled={busy} onClick={browse}>选择文件</button></div></label></> : <p className="runtime-path">{app?.path}</p>}
      <label className="form-field">搜索别名<textarea data-initial-focus aria-label="搜索别名" placeholder="每行一个，也可用逗号分隔。支持拼音搜索。" value={aliases} onChange={event => setAliases(event.target.value)}/></label>
      {!manual && !shellManaged && <label className="form-check"><input type="checkbox" checked={custom} onChange={event => setCustom(event.target.checked)}/>自定义启动参数和工作目录</label>}
      {shellManaged ? <p className="settings-hint">此入口由 Windows 管理启动，可设置搜索别名。</p> : <>
        <label className="form-field">启动参数<input aria-label="启动参数" disabled={!manual && !custom} value={args} onChange={event => setArgs(event.target.value)} placeholder="例如 --profile work"/></label>
        <label className="form-field">工作目录<input aria-label="工作目录" disabled={!manual && !custom} value={directory} onChange={event => setDirectory(event.target.value)} placeholder="留空使用程序所在目录"/></label>
        {!manual && <p className="settings-hint">取消自定义后保存，可恢复原快捷方式的启动方式。</p>}
      </>}
      {error && <p className="form-error" role="alert">{error}</p>}
      <div className="panel-actions"><button className="btn btn-primary" disabled={busy} onClick={save}>{busy ? "处理中…" : "保存应用"}</button><button className="btn" disabled={busy} onClick={onClose}>取消</button>
        {manual && config.manual_apps?.some(item => item.id === id) && <button className="btn danger" disabled={busy} onClick={() => {
          if (!removeConfirm) { setRemoveConfirm(true); return; }
          void run(async () => onSaved(await invoke<Config>("remove_manual_app", { appId: id })));
        }}>{removeConfirm ? "确认从 Luma 移除" : "移除手动应用"}</button>}</div>
      {removeConfirm && <p className="settings-hint">仅移除 Luma 中的手动记录，保留程序文件。</p>}
  </Dialog>;
}
