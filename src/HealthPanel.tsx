import { useState } from "react";
import { Dialog } from "./Dialog";
import { EntryIcon } from "./EntryIcon";
import type { AppEntry, EntryHealth } from "./types";
interface Props { apps: AppEntry[]; checks: EntryHealth[]; busy: boolean; error: string; onClose: () => void; onCheck: () => void; onRepair: (app: AppEntry) => void }
export function HealthPanel({ apps, checks, busy, error, onClose, onCheck, onRepair }: Props) {
  const [showUnchecked,setShowUnchecked]=useState(false);
  const byId=new Map(apps.map(app=>[app.id,app]));
  const issues=checks.filter(item=>item.state!=="unchecked");
  const visible=checks.filter(item=>showUnchecked||item.state!=="unchecked");
  return <Dialog title="入口检查" label="关闭入口检查" className="health-panel" onClose={onClose}>
    <p className="settings-hint">检查本机目标和工作目录。网页仅校验地址格式；系统入口与网络路径在启动时验证，不自动删除记录。</p>
    <div className="health-summary"><strong>{issues.length ? `${issues.length} 个入口需要处理` : "未发现本机入口问题"}</strong><button className="btn" disabled={busy} onClick={onCheck}>{busy ? "检查中…" : "重新检查"}</button></div>
    <label className="form-check"><input type="checkbox" checked={showUnchecked} onChange={event=>setShowUnchecked(event.target.checked)}/>显示未验证的系统与网络入口（{checks.length-issues.length}）</label>
    {error && <p role="alert" className="form-error">{error}</p>}
    <div className="health-list">{visible.map(check=>{
      const app=byId.get(check.id); if(!app)return null;
      return <div className={`health-row health-${check.state}`} key={app.id}><div className="health-entry-icon"><EntryIcon app={app}/></div><div className="health-entry-copy"><strong>{app.name}</strong><p>{check.message}</p><small title={app.path}>{app.path}</small></div>{check.state!=="unchecked" && <button className="btn" onClick={()=>onRepair(app)}>重新选择目标</button>}</div>;
    })}</div>
    <p className="settings-hint">修复时保留名称、别名、分类和固定顺序。</p>
  </Dialog>;
}
