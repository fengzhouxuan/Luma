import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CSSProperties, KeyboardEvent, PointerEvent } from "react";
import { canvasPng, clamp, contains, intersect, moveRegion, paint, rectangle, resizeRegion } from "./screenshotGeometry";
import type { Mark, Point, Rect, Scene, Tool } from "./screenshotGeometry";
import "./screenshot.css";

const tools: { id: Tool; icon: string; title: string }[] = [
  {id:"move",icon:"↖",title:"调整选区"}, {id:"arrow",icon:"↗",title:"箭头"},
  {id:"line",icon:"╱",title:"直线"}, {id:"rect",icon:"□",title:"矩形"},
  {id:"ellipse",icon:"○",title:"椭圆"}, {id:"pen",icon:"✎",title:"画笔"},
  {id:"highlight",icon:"▰",title:"高亮"}, {id:"text",icon:"T",title:"文字"},
  {id:"number",icon:"①",title:"序号"}, {id:"mosaic",icon:"▒",title:"马赛克"},
  {id:"cover",icon:"■",title:"纯色遮盖"},
];
const handles = ["nw", "n", "ne", "w", "e", "sw", "s", "se"];
const colors = ["#ff535f", "#ffcc56", "#70dfad", "#5daaff", "#ffffff", "#111827"];

type Mode = "region" | "window" | "monitor";
type Output = "copy" | "save" | "pin";
type Drag =
  | {kind:"new"; origin:Point; snap?:Rect}
  | {kind:"move"; origin:Point; base:Rect}
  | {kind:"resize"; handle:string; base:Rect}
  | {kind:"mark"; mark:Mark};

function sendImage(action: Output, png: ArrayBuffer, area?: Rect) {
  const headers: Record<string, string> = {"x-luma-action": action};
  if (area) headers["x-luma-region"] = JSON.stringify(area);
  return invoke<boolean>("screenshot_output", png, {headers});
}

function iconButton(text: string, label: string, action: () => void, active?: boolean) {
  return <button type="button" title={label} aria-label={label}
    aria-pressed={active} onClick={action}>{text}</button>;
}

export function ScreenshotEditor() {
  const [scene, setScene] = useState<Scene>();
  const [image, setImage] = useState<HTMLImageElement>();
  const [cursor, setCursor] = useState<Point>();
  const [area, setArea] = useState<Rect>();
  const [mode, setMode] = useState<Mode>("region");
  const [tool, setTool] = useState<Tool>("move");
  const [color, setColor] = useState(colors[0]);
  const [stroke, setStroke] = useState(4);
  const [marks, setMarks] = useState<Mark[]>([]);
  const [redo, setRedo] = useState<Mark[]>([]);
  const [draft, setDraft] = useState<Mark>();
  const [text, setText] = useState<{point: Point; value: string}>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [dragging, setDragging] = useState(false);
  const [viewport, setViewport] = useState(() => ({width: innerWidth, height: innerHeight}));
  const editor = useRef<HTMLElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const drag = useRef<Drag | undefined>(undefined);
  const saving = useRef(false);
  const shown = useRef(false);
  const selected = !!area && area.width >= 2 && area.height >= 2;
  const editingText = !!text;

  useEffect(() => {
    let disposed = false;
    let url: string | undefined;
    const resize = () => setViewport({ width: innerWidth, height: innerHeight });
    window.addEventListener("resize", resize);
    void Promise.all([invoke<Scene>("screenshot_scene"), invoke<ArrayBuffer>("screenshot_pixels")])
      .then(async ([layout, png]) => {
        if (disposed) return;
        const picture = new Image();
        url = URL.createObjectURL(new Blob([png], {type:"image/png"}));
        picture.src = url;
        await picture.decode();
        if (disposed) return;
        if (picture.naturalWidth !== layout.bounds.width || picture.naturalHeight !== layout.bounds.height) {
          throw new Error("截图尺寸与屏幕布局不一致");
        }
        setScene(layout);
        setImage(picture);
        if (layout.reuse_last && layout.last_area) setArea(layout.last_area);
        if (layout.cursor) setCursor({ x: layout.cursor[0], y: layout.cursor[1] });
      })
      .catch(reason => { if (!disposed) void invoke("screenshot_close", {error: String(reason)}); });
    return () => {
      disposed = true;
      window.removeEventListener("resize", resize);
      if (url) URL.revokeObjectURL(url);
    };
  }, []);

  useLayoutEffect(() => {
    if (!scene || !image || shown.current) return;
    shown.current = true;
    void invoke("screenshot_ready").catch(reason => invoke("screenshot_close", {error: String(reason)}));
  }, [scene, image]);

  useLayoutEffect(() => {
    if (canvas.current && image && scene && area?.width && area.height) {
      paint(canvas.current, image, scene.bounds, area, draft ? [...marks, draft] : marks);
    }
  }, [area, image, scene, marks, draft]);

  useLayoutEffect(() => {
    if (document.activeElement === document.body) editor.current?.focus({preventScroll: true});
  }, [scene, image, selected, editingText, busy]);

  const close = () => { if (!saving.current) void invoke("screenshot_close"); };
  const focusEditor = () => editor.current?.focus({preventScroll: true});

  async function output(action: Output) {
    if (!canvas.current || !area || area.width < 2 || area.height < 2 || saving.current) return;
    focusEditor();
    saving.current = true;
    setBusy(true);
    setError("");
    try {
      await sendImage(action, await canvasPng(canvas.current), area);
    } catch (reason) {
      setError(String(reason));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  }

  async function startLongScreenshot() {
    if (!area || saving.current) return;
    if (marks.length) {
      setError("请撤销标注后再开始长截图");
      return;
    }
    focusEditor();
    saving.current = true;
    setBusy(true);
    setError("");
    try {
      await invoke("screenshot_start_long", {area});
    } catch (reason) {
      setError(String(reason));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  }

  function undoMark() {
    const item = marks[marks.length - 1];
    if (!item) return;
    setMarks(marks.slice(0, -1));
    setRedo([...redo, item]);
  }

  function redoMark() {
    const item = redo[redo.length - 1];
    if (!item) return;
    setMarks([...marks, item]);
    setRedo(redo.slice(0, -1));
  }

  function append(mark: Mark) {
    setMarks(items => [...items, mark]);
    setRedo([]);
  }

  function selectMode(next: Mode) {
    if (selected) focusEditor();
    setMode(next);
    setArea(undefined);
    setTool("move");
    setMarks([]);
    setRedo([]);
    setError("");
  }

  function confirmArea(next: Rect) {
    focusEditor();
    setArea(next);
  }

  function closeText() {
    focusEditor();
    setText(undefined);
  }

  if (!scene || !image) return <main className="shot-loading">正在准备截图…</main>;

  const bounds = scene.bounds;
  const lastArea = scene.last_area;
  const sx = viewport.width / bounds.width;
  const sy = viewport.height / bounds.height;
  const pointFrom = (event: PointerEvent): Point => ({
    x: clamp(Math.round(event.clientX / sx + bounds.x), bounds.x, bounds.x + bounds.width),
    y: clamp(Math.round(event.clientY / sy + bounds.y), bounds.y, bounds.y + bounds.height),
  });
  const cssRect = (rect: Rect): CSSProperties => ({
    left: (rect.x - bounds.x) * sx, top: (rect.y - bounds.y) * sy,
    width: rect.width * sx, height: rect.height * sy,
  });
  const monitorAt = (point: Point) => scene.monitors.find(rect => contains(rect, point));
  const snapAt = (point: Point) => {
    const target = mode === "monitor" ? monitorAt(point) : scene.windows.find(rect => contains(rect, point));
    return target && intersect(bounds, target);
  };
  const hover = !area && cursor && mode !== "region" ? snapAt(cursor) : undefined;
  const region = area ?? hover;
  const anchor = area ? {x:area.x + area.width - 1, y:area.y + area.height - 1} : cursor;
  const monitor = (anchor && monitorAt(anchor))
    ?? (area && scene.monitors.find(item => intersect(item, area)))
    ?? scene.monitors.find(item => contains(item, {x:0,y:0}))
    ?? scene.monitors[0];

  function startDrag(event: PointerEvent<HTMLElement>) {
    if (event.button !== 0 || saving.current || text || (event.target as HTMLElement).closest("[data-shot-control]")) return;
    const point = pointFrom(event);
    event.currentTarget.focus({ preventScroll: true });
    const handle = (event.target as HTMLElement).dataset.handle;
    let action: Drag;
    if (area && (contains(area, point) || handle)) {
      if (handle) action = {kind:"resize", handle, base:area};
      else if (tool === "move") action = {kind:"move", origin:point, base:area};
      else if (tool === "text") {
        setText({point,value:""});
        return;
      } else if (tool === "number") {
        append({tool, points:[point], color, width:stroke, text:String(marks.filter(m => m.tool === "number").length + 1)});
        return;
      } else {
        const mark = {tool, points:[point], color, width:stroke};
        action = {kind:"mark", mark};
        setDraft(mark);
      }
    } else {
      const snap = mode === "region" ? undefined : snapAt(point);
      if (mode !== "region" && !snap) return;
      action = {kind:"new", origin:point, snap};
      setArea(snap ?? rectangle(point, point));
      setMarks([]);
      setRedo([]);
      setTool("move");
    }
    drag.current = action;
    event.currentTarget.setPointerCapture(event.pointerId);
    setDragging(true);
  }

  function updateMark(mark: Mark, point: Point): Mark {
    if (mark.tool !== "pen" && mark.tool !== "highlight") return {...mark, points:[mark.points[0],point]};
    const last = mark.points[mark.points.length - 1];
    return Math.hypot(last.x - point.x, last.y - point.y) >= 1 && mark.points.length < 12000
      ? {...mark, points:[...mark.points, point]} : mark;
  }

  function movePointer(event: PointerEvent<HTMLElement>) {
    const point = pointFrom(event);
    setCursor(point);
    const action = drag.current;
    if (!action) return;
    if (action.kind === "new") {
      setArea(action.snap ?? rectangle(action.origin, point));
    } else if (action.kind === "move") {
      setArea(moveRegion(action.base, point.x - action.origin.x, point.y - action.origin.y, bounds));
    } else if (action.kind === "resize") {
      setArea(resizeRegion(action.base, action.handle, point, bounds));
    } else {
      action.mark = updateMark(action.mark, point);
      setDraft(action.mark);
    }
  }

  function endDrag(event: PointerEvent<HTMLElement>) {
    if (!drag.current) return;
    movePointer(event);
    const action = drag.current;
    if (action.kind === "mark") {
      append(action.mark);
      setDraft(undefined);
    } else if (action.kind === "new" && !action.snap) {
      const next = rectangle(action.origin, pointFrom(event));
      if (next.width < 2 || next.height < 2 || !scene?.monitors.some(item => intersect(item, next))) {
        setArea(undefined);
      }
    }
    drag.current = undefined;
    setDragging(false);
  }

  function cancelDrag() {
    const action = drag.current;
    if (!action) return;
    if (action.kind === "new") setArea(undefined);
    else if (action.kind === "move" || action.kind === "resize") setArea(action.base);
    setDraft(undefined);
    drag.current = undefined;
    setDragging(false);
  }

  function onKey(event: KeyboardEvent<HTMLElement>) {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      if (text) closeText();
      else if (drag.current) cancelDrag();
      else close();
      return;
    }
    if ((event.target as HTMLElement).closest("input,textarea") || busy || dragging) return;
    const key = event.key.toLowerCase();
    if (event.ctrlKey && (key === "z" || key === "y")) {
      event.preventDefault();
      if (key === "y" || event.shiftKey) redoMark();
      else undoMark();
    } else if (event.ctrlKey && event.shiftKey && key === "l" && selected) {
      event.preventDefault();
      void startLongScreenshot();
    } else if (event.ctrlKey && key === "s" && area) {
      event.preventDefault();
      void output("save");
    } else if (event.altKey && event.key === "Enter" && hover) {
      event.preventDefault();
      confirmArea(hover);
    } else if ((event.key === "Enter" && !(event.target instanceof HTMLButtonElement) || event.ctrlKey && key === "c") && area) {
      event.preventDefault();
      void output("copy");
    } else if (area && event.key.startsWith("Arrow")) {
      event.preventDefault();
      const dx = Number(event.key === "ArrowRight") - Number(event.key === "ArrowLeft");
      const dy = Number(event.key === "ArrowDown") - Number(event.key === "ArrowUp");
      setArea(event.shiftKey ? resizeRegion(area,"se", {x:area.x + area.width + dx,y:area.y + area.height + dy},bounds)
        : moveRegion(area,dx,dy,bounds));
    }
  }

  const screen = cssRect(monitor);
  const sx0 = Number(screen.left);
  const sy0 = Number(screen.top);
  const sw = Number(screen.width);
  const sh = Number(screen.height);
  const barWidth = Math.min(610, sw - 24);
  const toolbarLeft = clamp(area ? (area.x + area.width - bounds.x) * sx - barWidth : sx0 + (sw - barWidth) / 2,
    sx0 + 12, sx0 + sw - barWidth - 12);
  const regionTop = area ? (area.y - bounds.y) * sy : 0;
  const regionBottom = area ? regionTop + area.height * sy : 0;
  const toolbarTop = selected
    ? clamp(regionBottom + 12 + 112 < sy0 + sh ? regionBottom + 12 : regionTop - 120, sy0 + 12, sy0 + sh - 120)
    : sy0 + 16;
  const magLeft = cursor && clamp((cursor.x - bounds.x) * sx + 24,sx0 + 8,sx0 + sw - 128);
  const magTop = cursor && clamp((cursor.y - bounds.y) * sy + 24,sy0 + 76,sy0 + sh - 124);

  return <main className={"shot-editor" + (selected && tool !== "move" ? " shot-drawing" : "")}
    tabIndex={-1} onKeyDown={onKey} ref={editor}
    onPointerDown={startDrag} onPointerMove={movePointer} onPointerUp={endDrag}
    onPointerCancel={cancelDrag} onLostPointerCapture={cancelDrag}
    onContextMenu={event => event.preventDefault()}>
    <img className="shot-desktop" src={image.src} alt="" draggable={false} />
    {!region && <div className="shot-dim" />}
    {region && <div className="shot-selection" style={cssRect(region)}>
      {selected && <canvas ref={canvas} aria-label="截图预览" />}
      {selected && tool === "move" && handles.map(handle =>
        <span key={handle} className={"shot-handle shot-" + handle} data-handle={handle} />)}
      {region.width > 0 && region.height > 0 && <span className="shot-size">{region.width} × {region.height}</span>}
    </div>}
    {!selected && !dragging && cursor && <div className="shot-magnifier" style={{left:magLeft,top:magTop}}>
      <div className="shot-magnifier-pixels" style={{
        backgroundImage:"url(" + image.src + ")",
        backgroundSize:bounds.width * 6 + "px " + bounds.height * 6 + "px",
        backgroundPosition:(56 - (cursor.x - bounds.x) * 6) + "px " + (43 - (cursor.y - bounds.y) * 6) + "px",
      }} />
      <span>{cursor.x}, {cursor.y}</span>
    </div>}
    {!dragging && <section className="shot-toolbar" style={{left:toolbarLeft,top:toolbarTop,width:barWidth}}
      onMouseDown={event => { if (event.target instanceof HTMLButtonElement) event.preventDefault(); }}
      data-shot-control aria-label="截图工具栏">
      {!selected ? <>
        <div className="shot-row shot-mode">
          {iconButton("区域", "区域框选", () => selectMode("region"), mode === "region")}
          {iconButton("窗口", "窗口截图", () => selectMode("window"), mode === "window")}
          {iconButton("单屏", "单屏截图", () => selectMode("monitor"), mode === "monitor")}
          {iconButton("全屏", "全部显示器", () => confirmArea(bounds), false)}
          {lastArea && iconButton("上次区域", "截取上次区域", () => confirmArea(lastArea), false)}
          {iconButton("关闭", "取消截图", close)}
        </div>
        <div className="shot-hint">{mode === "region"
          ? scene.reuse_last && !scene.last_area ? "上次区域不可用，请重新框选" : "拖动框选区域"
          : mode === "window" ? "点击高亮的窗口 · Alt+Enter 确认" : "点击要截取的显示器 · Alt+Enter 确认"} · Esc 取消</div>
      </> : <>
        <div className="shot-row shot-tools">
          {tools.map(item => <button type="button" key={item.id} title={item.title} aria-label={item.title}
            aria-pressed={tool === item.id} onClick={() => setTool(item.id)}>{item.icon}</button>)}
          <span className="shot-divider" />
          <button type="button" title="撤销 Ctrl+Z" aria-label="撤销" disabled={!marks.length} onClick={undoMark}>↶</button>
          <button type="button" title="重做 Ctrl+Y" aria-label="重做" disabled={!redo.length} onClick={redoMark}>↷</button>
        </div>
        <div className="shot-row shot-actions">
          <div className="shot-palette" aria-label="标注颜色">
            {colors.map(value => <button type="button" key={value} aria-label={"颜色 " + value}
              aria-pressed={color === value} style={{"--swatch":value} as CSSProperties}
              onClick={() => setColor(value)} />)}
          </div>
          <select aria-label="标注粗细" value={stroke} onChange={event => setStroke(Number(event.target.value))}>
            <option value={3}>细</option><option value={4}>中</option><option value={7}>粗</option>
          </select>
          <div className="shot-output">
            <button type="button" onClick={() => selectMode("region")} disabled={busy}>重选</button>
            <button type="button" title="滚动长截图 · Ctrl+Shift+L" onClick={() => void startLongScreenshot()} disabled={busy}>长截图</button>
            <button type="button" onClick={() => void output("pin")} disabled={busy}>贴图</button>
            <button type="button" onClick={() => void output("save")} disabled={busy}>保存</button>
            <button type="button" className="shot-primary" title="复制并退出 · Enter" onClick={() => void output("copy")} disabled={busy}>{busy ? "处理中…" : "复制"}</button>
            <button type="button" title="Esc 取消" aria-label="取消截图" onClick={close} disabled={busy}>×</button>
          </div>
        </div>
      </>}
      {error && <div className="shot-error" role="alert">{error}</div>}
    </section>}
    {text && <form className="shot-text-form" data-shot-control style={{
      left:clamp((text.point.x - bounds.x) * sx, sx0 + 8, sx0 + sw - 300),
      top:clamp((text.point.y - bounds.y) * sy, sy0 + 8, sy0 + sh - 65),
    }} onSubmit={event => {
      event.preventDefault();
      if (text.value.trim()) append({tool:"text",points:[text.point],color,width:stroke,text:text.value.trim()});
      closeText();
    }}>
      <input autoFocus aria-label="截图文字" placeholder="输入标注文字" value={text.value} maxLength={300}
        onChange={event => setText({point:text.point,value:event.target.value})} />
      <button type="submit">确定</button>
      <button type="button" onClick={closeText}>取消</button>
    </form>}
  </main>;
}

export function ScreenshotToast() {
  useLayoutEffect(() => { void invoke("screenshot_toast_ready").catch(() => {}); }, []);
  return <div className="shot-toast" role="status">✓ 已复制截图</div>;
}

export function ScreenshotPin() {
  const [png, setPng] = useState<ArrayBuffer>();
  const [source, setSource] = useState("");
  const [error, setError] = useState("");
  const zooming = useRef(false);
  useEffect(() => {
    let disposed = false;
    let url: string | undefined;
    void invoke<ArrayBuffer>("screenshot_pixels").then(data => {
      if (disposed) return;
      url = URL.createObjectURL(new Blob([data], {type:"image/png"}));
      setPng(data);
      setSource(url);
    }).catch(reason => { if (!disposed) setError(String(reason)); });
    return () => {
      disposed = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, []);
  const close = () => void invoke("screenshot_close");
  function pinOutput(action: "copy" | "save") {
    if (png) void sendImage(action, png).catch(reason => setError(String(reason)));
  }
  return <main className="shot-pin" tabIndex={-1}
    ref={element => { if (element && document.activeElement === document.body) element.focus(); }}
    onKeyDown={event => {
    if (event.key === "Escape") close();
    if (event.ctrlKey && event.key.toLowerCase() === "c") pinOutput("copy");
  }} onWheel={event => {
    if (!event.deltaY || zooming.current) return;
    zooming.current = true;
    void invoke("screenshot_zoom_pin", {zoomIn:event.deltaY < 0})
      .catch(reason => setError(String(reason))).finally(() => {zooming.current = false;});
  }}>
    {source && <img src={source} alt="贴图" draggable={false} onPointerDown={event => {
      if (event.button === 0) void getCurrentWindow().startDragging();
    }}/>}
    <div className="shot-pin-controls">
      <button onClick={() => pinOutput("copy")} title="复制图片">复制</button>
      <button onClick={() => pinOutput("save")} title="保存图片">保存</button>
      <button onClick={close} aria-label="关闭贴图" title="关闭贴图">×</button>
    </div>
    {error && <div className="shot-error">{error}</div>}
  </main>;
}
