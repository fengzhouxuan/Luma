import { useState } from "react";
import { Dialog } from "./Dialog";
import { invoke } from "@tauri-apps/api/core";
import type { AppEntry, Config, ManualApp, LaunchOverride, EntryKind } from "./types";

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
  const kind = manual?.kind ?? "app";
  const hasArguments = kind === "app" || kind === "script";
  const run = async (task: () => Promise<void>) => { setBusy(true); setError(""); try { await task(); } catch (error) { setError(String(error)); } finally { setBusy(false); } };
  const browse = () => void run(async () => {
    const next = await invoke<ManualApp | null>("pick_manual_app", { kind });
    if (next) { setManual({ ...next, id, name: manual?.name || next.name }); setArgs(next.arguments); setDirectory(next.working_dir ?? ""); }
  });
  const save = () => void run(async () => {
    const values = aliases.split(/[\n,，]+/);
    let next: Config;
    if (manual) {
      next = await invoke<Config>("save_manual_app", { item: { ...manual, name: manual.name.trim(), path: manual.path.trim(), arguments: hasArguments ? args : "", working_dir: hasArguments ? directory.trim() || null : null }, aliases: values });
    } else {
      const launch: LaunchOverride | null = custom && !shellManaged ? { arguments: args, working_dir: directory.trim() || null } : null;
      next = await invoke<Config>("save_app_options", { appId: id, aliases: values, launch });
    }
    onSaved(next);
  });
  return <Dialog title={manual ? "快捷入口" : `应用设置 · ${app?.name}`} label="关闭应用设置" className="editor-panel" busy={busy} onClose={onClose}>
      {manual ? <><label className="form-field">入口类型<select aria-label="入口类型" disabled={busy} value={kind} onChange={event => {
        setManual({ ...manual, kind: event.target.value as EntryKind, path: "", arguments: "", working_dir: null, shortcut_path: null });
        setArgs(""); setDirectory(""); setError("");
      }}><option value="app">应用</option><option value="folder">文件夹</option><option value="web">网页</option><option value="script">脚本</option></select></label>
        <label className="form-field">入口名称<input data-initial-focus aria-label="入口名称" value={manual.name} onChange={event => setManual({ ...manual, name: event.target.value })}/></label>
        <label className="form-field">{kind === "web" ? "网页地址" : kind === "folder" ? "文件夹路径" : kind === "script" ? "脚本路径" : "程序路径"}<div className="settings-row"><input aria-label="入口目标" readOnly={kind === "app"} value={manual.path} onChange={event => setManual({ ...manual, path: event.target.value })} placeholder={kind === "web" ? "https://example.com" : "选择本机目标"}/>{kind !== "web" && <button className="btn" disabled={busy} onClick={browse}>{kind === "folder" ? "选择文件夹" : "选择文件"}</button>}</div></label>
        {kind === "web" && <p className="settings-hint">使用系统默认浏览器打开网页。</p>}
        {kind === "script" && <p className="settings-hint">支持 .bat、.cmd、.ps1；PowerShell 沿用系统执行策略。</p>}
      </> : <p className="runtime-path">{app?.path}</p>}
      <label className="form-field">搜索别名<textarea data-initial-focus={!manual || undefined} aria-label="搜索别名" placeholder="每行一个，也可用逗号分隔。支持拼音搜索。" value={aliases} onChange={event => setAliases(event.target.value)}/></label>
      {!manual && !shellManaged && <label className="form-check"><input type="checkbox" checked={custom} onChange={event => setCustom(event.target.checked)}/>自定义启动参数和工作目录</label>}
      {shellManaged ? <p className="settings-hint">此入口由 Windows 管理启动，可设置搜索别名。</p> : hasArguments && <>
        <label className="form-field">启动参数<input aria-label="启动参数" disabled={!manual && !custom} value={args} onChange={event => setArgs(event.target.value)} placeholder="例如 --profile work"/></label>
        <label className="form-field">工作目录<input aria-label="工作目录" disabled={!manual && !custom} value={directory} onChange={event => setDirectory(event.target.value)} placeholder="留空使用程序所在目录"/></label>
        {!manual && <p className="settings-hint">取消自定义后保存，可恢复原快捷方式的启动方式。</p>}
      </>}
      {error && <p className="form-error" role="alert">{error}</p>}
      <div className="panel-actions"><button className="btn btn-primary" disabled={busy} onClick={save}>{busy ? "处理中…" : manual ? "保存入口" : "保存应用"}</button><button className="btn" disabled={busy} onClick={onClose}>取消</button>
        {manual && config.manual_apps?.some(item => item.id === id) && <button className="btn danger" disabled={busy} onClick={() => {
          if (!removeConfirm) { setRemoveConfirm(true); return; }
          void run(async () => onSaved(await invoke<Config>("remove_manual_app", { appId: id })));
        }}>{removeConfirm ? "确认从 Luma 移除" : "移除入口"}</button>}</div>
      {removeConfirm && <p className="settings-hint">仅移除 Luma 中的记录，保留目标文件。</p>}
  </Dialog>;
}
