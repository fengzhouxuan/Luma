import { useState } from "react";
import type { AppEntry, Config, Diagnostics } from "./types";

interface Props {
  apps: AppEntry[];
  config: Config | null;
  snapshot: Diagnostics | null;
  initialId: string | null;
  onClose: () => void;
  onRefresh: () => void;
  onPin: (app: AppEntry) => void;
  onHide: (app: AppEntry) => void;
}

export function DiagnosticsPanel({ apps, config, snapshot, initialId, onClose, onRefresh, onPin, onHide }: Props) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(initialId);
  const q = query.toLowerCase().trim();
  const matches = apps.filter((app) => `${app.name} ${app.path} ${app.source}`.toLowerCase().includes(q));
  const app = matches.find((entry) => entry.id === selected) ?? matches[0];
  const icon = snapshot?.events.find((event) => event.stage === "图标" && (event.path === app?.shortcut_path || event.path === app?.path));
  const lastLaunch = snapshot?.events.find((event) => event.stage === "启动" && event.app_id === app?.id);
  const report = snapshot?.report;
  const packaged = app?.path.startsWith("shell:AppsFolder\\") ?? false;
  return <div className="settings-overlay" onClick={onClose}>
    <section className="diagnostics-panel" onClick={(event) => event.stopPropagation()} aria-label="应用诊断">
      <div className="settings-header"><span>应用诊断 <small>Luma {snapshot?.version ?? ""}</small></span>
        <div className="panel-actions"><button className="btn btn-sm" onClick={onRefresh}>刷新诊断</button>
          <button className="settings-close" aria-label="关闭诊断" onClick={onClose}>×</button></div></div>
      <div className="diagnostics-summary">已发现 {apps.length} 个应用 · {report?.full ? "全量扫描" : "增量检查"} {report?.duration_ms ?? 0}ms
        <br/>监听 {report?.watched_directories ?? 0} 个目录、{report?.watched_registry_keys ?? 0} 个注册表位置
        {report?.rescanned.length ? ` · 本轮更新：${report.rescanned.join("、")}` : " · 本轮复用扫描缓存"}</div>
      <input className="diagnostics-search" aria-label="搜索诊断应用" placeholder="搜索应用名称、路径或来源" value={query} onChange={(event) => setQuery(event.target.value)}/>
      <div className="diagnostics-body">
        <div className="diagnostics-list">{matches.map((entry) => <button key={entry.id} className={entry.id === app?.id ? "active" : ""}
          onClick={() => setSelected(entry.id)}><strong>{entry.name}</strong><span>{entry.hidden_reason ?? entry.source}</span></button>)}
          {matches.length === 0 && <p>没有匹配的已发现应用，请查看下方扫描记录。</p>}</div>
        <div className="diagnostics-detail">{app ? <>
          <h3>{app.name}</h3>
          <div className="panel-actions"><button className="btn" onClick={() => onPin(app)}>{config?.pinned_apps?.includes(app.id) ? "取消置顶" : "置顶应用"}</button>
            <button className="btn" onClick={() => onHide(app)}>{config?.hidden_apps?.includes(app.id) ? "取消手动隐藏" : "手动隐藏"}</button></div>
          <dl>{[
            ["扫描来源", app.source], ["来源位置", app.source_path], ["程序路径", app.path],
            ["快捷方式", app.shortcut_path ?? (packaged ? "通过 Windows 应用标识启动" : "无，直接启动程序")], ["启动参数", app.arguments || "无"],
            ["工作目录", app.working_dir ?? (packaged ? "由 Windows 应用模型管理" : "程序所在目录")], ["分类", app.category],
            ["过滤原因", app.hidden_reason ?? "未过滤"], ["图标状态", icon?.message ?? (app.icon ? "已加载" : "尚未加载，或等待重试")],
            ["最近启动", lastLaunch?.message ?? "本次运行暂无启动记录"],
          ].map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
        </> : <p>尚无应用记录，可点击刷新应用重新扫描。</p>}</div>
      </div>
      <details className="diagnostics-events"><summary>最近扫描、监听与图标记录（{snapshot?.events.length ?? 0}）</summary>
        <div>{snapshot?.events.map((event, index) => <p key={index}><strong>{event.stage}</strong> · {new Date(event.time * 1000).toLocaleTimeString()} · {event.message}<br/><small>{event.path}</small></p>)}</div></details>
    </section>
  </div>;
}
