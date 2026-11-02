import { useEffect, useImperativeHandle, useRef, useState, type KeyboardEvent, type Ref } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";
import { fileShortcut, PreviewQueue, previewError } from "./filePreview";
import { fileQuery, fileSize, SearchRevision, type FileEntry, type FileFilter, type FileSearchResult } from "./fileSearch";
import { UiIcon } from "./UiIcon";
import "./fileSearch.css";

export interface FileSearchHandle { onKeyDown(event: KeyboardEvent): boolean; closePreview(): boolean; refresh(): void }
function EverythingStartupHint() {
  return <>
    <p>文件搜索需要 Everything 客户端在后台运行，仅启动 Everything 服务还不够。</p>
    <p>建议在 Everything 的「工具 → 选项 → 常规」中开启客户端随系统启动。下次登录 Windows 后会在后台启动，不弹搜索窗口。</p>
  </>;
}
export function FileSearch({ query, composing, suspended, ref, onOpened }: {
  query: string; composing: boolean; suspended: boolean; ref: Ref<FileSearchHandle>; onOpened(): void;
}) {
  const [filter, setFilter] = useState<FileFilter>("all");
  const [offset, setOffset] = useState(0);
  const [refresh, setRefresh] = useState(0);
  const [response, setResponse] = useState<FileSearchResult | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [selected, setSelected] = useState(0);
  const [actionBusy, setActionBusy] = useState(false);
  const [startupUntil, setStartupUntil] = useState(0);
  const [previewing, setPreviewing] = useState(false);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [previewName, setPreviewName] = useState("");
  const [quicklookMissing, setQuicklookMissing] = useState(false);
  const previewWanted = useRef(false);
  const previewPending = useRef(false);
  const previewRequest = useRef(0);
  const skipNextFocusRefresh = useRef(false);
  const previewOwner = useRef(crypto.randomUUID());
  const previewRevisions = useRef(new SearchRevision());
  const previewQueue = useRef<PreviewQueue | null>(null);
  if (!previewQueue.current) previewQueue.current = new PreviewQueue(id => id
    ? invoke("preview_file", { id, sessionId: previewOwner.current, requestId: previewRequest.current })
    : invoke("close_file_preview", { sessionId: previewOwner.current }));
  const revisions = useRef(new SearchRevision());
  const resultsRef = useRef<HTMLUListElement>(null);
  const search = fileQuery(query, filter);
  const [previousSearch, setPreviousSearch] = useState(search);
  // Reset before committing, avoiding an intermediate query at the old offset.
  if (search !== previousSearch) { setPreviousSearch(search); setOffset(0); }
  const signature = JSON.stringify([search, offset, refresh]);
  const [loadedSignature, setLoadedSignature] = useState("");
  const current = signature === loadedSignature && !composing;
  const items = current ? response?.items ?? [] : [];
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const focusSelected = () => {
    if (document.hasFocus()) resultsRef.current?.querySelector<HTMLButtonElement>(`[data-index="${selectedRef.current}"] .file-open`)?.focus({ preventScroll: true });
  };
  const changePreview = async (id: string | null) => {
    const revision = previewRevisions.current.next();
    previewRequest.current = revision;
    if (id) setPreviewName(items.find(item => item.id === id)?.name ?? "");
    previewWanted.current = id !== null; previewPending.current = true;
    setPreviewing(id !== null); setPreviewBusy(true); setQuicklookMissing(false); setError("");
    try {
      const applied = await previewQueue.current!.submit(id);
      if (applied && !id && previewRevisions.current.isCurrent(revision)) focusSelected();
    } catch (reason) {
      if (!previewRevisions.current.isCurrent(revision)) return;
      const failure = previewError(reason); setError(failure.message); setQuicklookMissing(failure.missing);
      if (id) {
        try { await previewQueue.current!.submit(null); if (previewRevisions.current.isCurrent(revision)) { previewWanted.current = false; setPreviewing(false); } }
        catch { if (previewRevisions.current.isCurrent(revision)) { previewWanted.current = true; setPreviewing(true); } }
      } else { previewWanted.current = true; setPreviewing(true); }
    } finally {
      if (previewRevisions.current.isCurrent(revision)) { previewPending.current = false; setPreviewBusy(false); }
    }
  };
  const closePreview = () => {
    if (!previewWanted.current && !previewPending.current) return false;
    void changePreview(null); return true;
  };
  useEffect(() => {
    let disposed = false; let stop: (() => void) | undefined;
    void listen<{ session_id: string; request_id: number }>("quicklook-preview-closed", event => {
      if (disposed || event.payload.session_id !== previewOwner.current || !previewRevisions.current.isCurrent(event.payload.request_id)) return;
      if (!document.hasFocus()) skipNextFocusRefresh.current = true;
      previewRevisions.current.next(); previewWanted.current = false; previewPending.current = false;
      setPreviewing(false); setPreviewBusy(false); focusSelected();
      void previewQueue.current!.submit(null).catch(() => {});
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; });
    return () => {
      disposed = true; stop?.(); previewRevisions.current.next();
      void previewQueue.current!.submit(null).catch(() => {});
    };
  }, []);
  useEffect(() => { closePreview(); }, [search, offset, suspended, composing]);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void getCurrentWindow().onFocusChanged(({ payload: focused }) => {
      if (focused && !disposed) {
        const returningFromPreview = previewWanted.current || previewPending.current || skipNextFocusRefresh.current;
        skipNextFocusRefresh.current = false;
        if (!returningFromPreview) setRefresh(value => value + 1);
      }
      if (!focused && !disposed) setStartupUntil(0);
    }).then(unlisten => { if (disposed) unlisten(); else stop = unlisten; })
      .catch(reason => console.error("监听文件搜索重连失败", reason));
    return () => { disposed = true; stop?.(); };
  }, []);

  useEffect(() => {
    const revision = revisions.current.next();
    const requestId = crypto.randomUUID();
    setBusy(true); setError(""); setNotice(""); setSelected(0);
    if (composing || suspended) return;
    const timer = window.setTimeout(() => {
      void invoke<FileSearchResult>("search_files", { query: search, offset, requestId }).then(result => {
        if (!revisions.current.isCurrent(revision)) return;
        setResponse(result); setLoadedSignature(signature); setBusy(false);
      }).catch(reason => {
        if (revisions.current.isCurrent(revision)) { setError(String(reason)); setBusy(false); }
      });
    }, search ? 200 : 0);
    return () => {
      revisions.current.next(); window.clearTimeout(timer);
      void invoke("cancel_file_search", { requestId });
    };
  }, [search, offset, refresh, composing, suspended, signature]);
  // A successful process launch can precede the IPC server becoming available.
  // Retry briefly after an explicit Start, and stop on success, blur or timeout.
  useEffect(() => {
    if (!startupUntil || busy || !current || suspended || composing) return;
    if (response?.state === "ready" || Date.now() >= startupUntil) { setStartupUntil(0); return; }
    const timer = window.setTimeout(() => setRefresh(value => value + 1), Math.min(750, startupUntil - Date.now()));
    return () => window.clearTimeout(timer);
  }, [startupUntil, busy, current, suspended, composing, response]);
  useEffect(() => { resultsRef.current?.querySelector(`[data-index="${selected}"]`)?.scrollIntoView({ block: "nearest" }); }, [selected]);
  useEffect(() => { if (resultsRef.current) resultsRef.current.scrollTop = 0; }, [response?.state]);
  useEffect(() => { if (!notice) return; const timer = window.setTimeout(() => setNotice(""), 3000); return () => window.clearTimeout(timer); }, [notice]);

  const action = async (entry: FileEntry, operation: "open" | "reveal" | "copy") => {
    if (!current || busy || actionBusy) return;
    setActionBusy(true); setError("");
    try {
      await invoke("file_action", { id: entry.id, action: operation });
      if (operation === "copy") setNotice("已复制完整路径");
      else onOpened();
    } catch (reason) { setError(String(reason)); }
    finally { setActionBusy(false); }
  };
  useImperativeHandle(ref, () => ({
    refresh() { setRefresh(value => value + 1); },
    closePreview,
    onKeyDown(event) {
      if (composing || suspended || !current || busy || actionBusy) return false;
      const target = event.target instanceof HTMLElement ? event.target : null;
      const context = target?.closest(".file-open") ? "result" : target?.matches('input[aria-label="搜索文件"]') ? "search" : "other";
      // A held/modified Space must never fall through to the native button click
      // behavior, which would open the selected file in its associated application.
      if (context === "result" && event.key === " " && (event.repeat || event.shiftKey || event.ctrlKey || event.altKey || event.metaKey)) { event.preventDefault(); event.stopPropagation(); return true; }
      const shortcut = fileShortcut({ ...event, isComposing: event.nativeEvent.isComposing }, context);
      if (!shortcut || !items.length) return false;
      event.preventDefault(); event.stopPropagation();
      if (shortcut === "next" || shortcut === "previous") {
        const next = context === "search" ? selected : Math.max(0, Math.min(items.length - 1, selected + (shortcut === "next" ? 1 : -1)));
        setSelected(next);
        const row = resultsRef.current?.querySelector<HTMLButtonElement>(`[data-index="${next}"] .file-open`);
        row?.focus({ preventScroll: true });
        row?.scrollIntoView({ block: "nearest" });
        if (previewWanted.current) void changePreview(items[next].id);
      } else if (shortcut === "preview") {
        void changePreview(previewWanted.current ? null : items[selected].id);
      } else { closePreview(); void action(items[selected], shortcut === "reveal" ? "reveal" : shortcut === "copy" ? "copy" : "open"); }
      return true;
    },
  }));
  const start = async () => {
    setError(""); setStartupUntil(Date.now() + 15_000);
    try { await invoke("start_everything"); setRefresh(value => value + 1); }
    catch (reason) { setStartupUntil(0); setError(String(reason)); }
  };
  const status = error || (current ? response?.message : "");
  const download = async () => {
    try { await invoke("open_everything_download"); }
    catch (reason) { setError(`打开官方下载页失败：${reason}`); }
  };
  return <section className="file-search-panel" aria-label="文件搜索">
    <div className="file-search-toolbar">
      <div className="file-filters" role="group" aria-label="文件类型">
        {([["all", "全部"], ["files", "文件"], ["folders", "文件夹"]] as const).map(([value, label]) =>
          <button className="btn" key={value} aria-pressed={filter === value} onClick={() => { setFilter(value); setOffset(0); }}>{label}</button>)}
      </div>
      <span className="file-provider" title="使用本机 Everything 索引，搜索词和结果不上传">{response?.version ? `Everything ${response.version}` : "Everything"}</span>
      <button className="file-icon-button" aria-label="刷新文件搜索" title="刷新文件搜索" onClick={() => setRefresh(value => value + 1)}><UiIcon name="refresh"/></button>
    </div>
    <div className="file-message" role="status" aria-live="polite">
      {notice || (startupUntil && response?.state === "unavailable" ? "正在等待 Everything 启动并连接…" : status) || (busy ? "正在搜索…" : query.trim() ? `${response?.total.toLocaleString() ?? 0} 个结果` : "输入文件名或路径，搜索本机文件和文件夹")}
    </div>
    {(error || (current && response?.state === "error") || (current && response?.state === "indexing")) && <button className="btn file-retry" onClick={() => setRefresh(value => value + 1)}>重试</button>}
    {quicklookMissing && <a className="quicklook-download" href="https://github.com/QL-Win/QuickLook/releases" onClick={event => { event.preventDefault(); void invoke("open_quicklook_download").catch(reason => setError(String(reason))); }}>安装 QuickLook：打开官方下载页</a>}
    {(previewing || previewBusy) && <div className="file-preview-status" role="status"><span>{previewBusy ? previewing ? "正在连接 QuickLook…" : "正在关闭预览…" : `QuickLook 预览中：${previewName} · ↑↓ 切换 · 空格 / Esc 关闭`}</span><button className="btn" onClick={() => void changePreview(null)}>关闭预览</button></div>}
    <ul className="file-results" aria-label="文件搜索结果" ref={resultsRef} aria-busy={busy}>
      {response?.state !== "unavailable" && <li className="file-connection-help">
        <details><summary><UiIcon name="info"/><span>Everything 连接与自启说明</span></summary><div><EverythingStartupHint/></div></details>
      </li>}
      {(current || startupUntil > 0) && response?.state === "unavailable" && <li className="file-setup">
        <strong>连接 Everything 客户端后即可搜索文件</strong>
        <p>支持 1.4 / 1.5 普通版，请勿选择 Lite 版。安装后启动客户端，等待首次索引完成即可使用。</p>
        <a href="https://www.voidtools.com/downloads/" target="_blank" rel="noreferrer" onClick={event => { event.preventDefault(); void download(); }}>官方下载：https://www.voidtools.com/downloads/</a>
        <p>安装目录不限。回到 Luma 会自动重新连接，无需重启；便携版可先手动运行 Everything。</p>
        <EverythingStartupHint/>
        <div className="file-connection-actions"><button className="btn" disabled={startupUntil > 0} onClick={() => void start()}>{startupUntil ? "正在启动 Everything…" : "已安装，启动 Everything"}</button><button className="btn" onClick={() => setRefresh(value => value + 1)}>重新连接</button></div>
      </li>}
      {items.map((entry, index) => <li key={entry.id} className={`file-result${index === selected ? " selected" : ""}`} data-index={index}>
        <button className="file-open" disabled={actionBusy} onFocus={() => setSelected(index)} onClick={() => void action(entry, "open")} aria-label={`打开 ${entry.name}`} title={entry.path}>
          <span className={`file-type-icon${entry.is_dir ? " is-folder" : ""}`}><UiIcon name={entry.is_dir ? "folder" : "file"}/></span>
          <span className="file-result-copy"><strong>{entry.name}</strong><small>{entry.path}</small></span>
          <span className="file-size">{entry.is_dir ? "文件夹" : fileSize(entry.size)}</span>
        </button>
        <button className="file-icon-button" aria-label={`定位 ${entry.name}`} title="在资源管理器中定位" disabled={actionBusy} onClick={() => void action(entry, "reveal")}><UiIcon name="folder"/></button>
        <button className="file-icon-button" aria-label={`预览 ${entry.name}`} title="QuickLook 预览（文件结果获得焦点后按空格）" disabled={actionBusy} onClick={() => { setSelected(index); resultsRef.current?.querySelector<HTMLButtonElement>(`[data-index="${index}"] .file-open`)?.focus({ preventScroll: true }); void changePreview(entry.id); }}><UiIcon name="preview"/></button>
        <button className="file-icon-button" aria-label={`复制 ${entry.name} 的路径`} title="复制完整路径" disabled={actionBusy} onClick={() => void action(entry, "copy")}><UiIcon name="copy"/></button>
      </li>)}
      {!busy && current && response?.state === "ready" && items.length === 0 && <li className="file-empty"><UiIcon name="search"/><p>{query.trim() ? "没有找到匹配的文件" : "文件搜索由 Everything 提供"}</p><small>{query.trim() ? "可尝试缩短关键词，或检查 Everything 的索引范围" : "支持中文、路径、通配符和 ext:pdf 等 Everything 搜索语法"}</small></li>}
    </ul>
    <div className="file-search-footer">
      <span className="file-keyboard-hints">↑↓ 选择并聚焦结果 · 空格预览 · Enter 打开 · Shift+Enter 定位 · Ctrl+Shift+C 复制路径</span>
      <div className="file-pagination">
        <button className="btn" disabled={busy || offset === 0} onClick={() => setOffset(value => Math.max(0, value - 50))}>上一页</button>
        <span>{current && items.length ? `${offset + 1}–${offset + items.length}` : "—"}</span>
        <button className="btn" disabled={busy || !current || !response || offset + items.length >= response.total || offset >= 100000} onClick={() => setOffset(value => value + 50)}>下一页</button>
      </div>
    </div>
  </section>;
}
