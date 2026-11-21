import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import type { KeyboardEvent } from "react";
import "./screenshot.css";

type Status = {
  phase: "starting" | "scrolling" | "finishing" | "preview" | "failed";
  width: number;
  height: number;
  frames: number;
  note: string | null;
};

export function LongScreenshot() {
  const [status, setStatus] = useState<Status>();
  const [source, setSource] = useState("");
  const [top, setTop] = useState(0);
  const [bottom, setBottom] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const preview = status?.phase === "preview";

  useEffect(() => {
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    async function update() {
      try {
        const next = await invoke<Status>("screenshot_long_status");
        if (stopped) return;
        setStatus(next);
        if (next.phase !== "preview" && next.phase !== "failed") timer = setTimeout(update, 220);
      } catch (reason) {
        if (!stopped) {
          setError(String(reason));
          timer = setTimeout(update, 500);
        }
      }
    }
    void update();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);

  useEffect(() => {
    if (!preview) return;
    let stopped = false;
    let url: string | undefined;
    void invoke<ArrayBuffer>("screenshot_long_preview").then(png => {
      if (stopped) return;
      url = URL.createObjectURL(new Blob([png], {type: "image/png"}));
      setSource(url);
    }).catch(reason => { if (!stopped) setError(String(reason)); });
    return () => {
      stopped = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [preview]);

  function close() {
    if (!busy) void invoke("screenshot_long_cancel").catch(reason => setError(String(reason)));
  }

  function finish() {
    if (status?.phase !== "scrolling") return;
    setStatus({...status, phase: "finishing"});
    void invoke("screenshot_long_finish").catch(reason => setError(String(reason)));
  }

  async function output(action: "copy" | "save") {
    if (!source || busy) return;
    setBusy(true);
    setError("");
    try {
      await invoke("screenshot_long_output", {action, top, bottom});
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function onKey(event: KeyboardEvent<HTMLElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (preview && event.ctrlKey && event.key.toLowerCase() === "s") {
      event.preventDefault();
      void output("save");
    } else if (preview && event.key === "Enter" && event.target === event.currentTarget) {
      event.preventDefault();
      void output("copy");
    }
  }

  if (preview && status) {
    const height = status.height - top - bottom;
    const crop = (value: string, other: number) => Math.max(0, Math.min(status.height - other - 1, Math.trunc(Number(value) || 0)));
    return <main className="shot-long shot-long-review" tabIndex={-1} onKeyDown={onKey}
      ref={element => { if (element && document.activeElement === document.body) element.focus(); }}>
      <header className="shot-long-header">
        <h1>长截图预览</h1>
        <span aria-live="polite">{status.width} × {height} 像素</span>
        <button type="button" aria-label="关闭预览" title="关闭预览" onClick={close} disabled={busy}>×</button>
      </header>
      {status.note && <p className="shot-long-note">{status.note}。预览显示已拼接的内容。</p>}
      <div className="shot-long-preview" tabIndex={0} aria-label="长截图预览图">
        {source ? <div className="shot-long-crop" style={{width: status.width, aspectRatio: status.width + " / " + height}}>
          <img src={source} alt="裁剪后的长截图" style={{top: (-100 * top / height) + "%"}} />
        </div> : <span>正在生成预览…</span>}
      </div>
      {error && <p className="shot-error" role="alert">{error}</p>}
      <footer className="shot-long-actions">
        <label>顶部裁剪<input type="number" min="0" max={status.height - bottom - 1} value={top} disabled={busy}
          onChange={event => setTop(crop(event.target.value, bottom))} /></label>
        <label>底部裁剪<input type="number" min="0" max={status.height - top - 1} value={bottom} disabled={busy}
          onChange={event => setBottom(crop(event.target.value, top))} /></label>
        <button type="button" onClick={() => void output("save")} disabled={!source || busy}>保存</button>
        <button type="button" className="shot-primary" onClick={() => void output("copy")} disabled={!source || busy}>复制</button>
      </footer>
    </main>;
  }

  const message = error || status?.note || (status?.phase === "scrolling"
    ? "将鼠标移到框选区域，缓慢向下滚动"
    : status?.phase === "finishing" ? "正在结束采集…" : "正在准备采集…");
  return <main className="shot-long shot-long-control" onKeyDown={onKey}>
    <header className="shot-long-header">
      <h1>长截图</h1>
      {status?.height ? <span>{status.width} × {status.height} · {status.frames} 段</span> : null}
    </header>
    <p className={error || status?.note ? "shot-long-note" : ""} role="status">{message}</p>
    <div className="shot-long-actions">
      <button type="button" onClick={close}>取消</button>
      <button type="button" className="shot-primary" onClick={finish} disabled={status?.phase !== "scrolling"}>完成</button>
    </div>
  </main>;
}
