export interface Point { x: number; y: number }
export interface Rect extends Point { width: number; height: number }
export interface Scene {
  bounds: Rect;
  monitors: Rect[];
  windows: Rect[];
  cursor: [number, number] | null;
  last_area: Rect | null;
  reuse_last: boolean;
}
export type Tool = "move" | "arrow" | "line" | "rect" | "ellipse" | "pen" | "highlight" | "text" | "number" | "mosaic" | "cover";
export interface Mark {
  tool: Tool;
  points: Point[];
  color: string;
  width: number;
  text?: string;
}

export const clamp = (value: number, low: number, high: number) => Math.max(low, Math.min(high, value));
export function rectangle(a: Point, b: Point): Rect {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), width: Math.abs(a.x - b.x), height: Math.abs(a.y - b.y) };
}
export function contains(rect: Rect, point: Point): boolean {
  return point.x >= rect.x && point.x < rect.x + rect.width && point.y >= rect.y && point.y < rect.y + rect.height;
}
export function intersect(a: Rect, b: Rect): Rect | undefined {
  const x = Math.max(a.x, b.x);
  const y = Math.max(a.y, b.y);
  const width = Math.min(a.x + a.width, b.x + b.width) - x;
  const height = Math.min(a.y + a.height, b.y + b.height) - y;
  return width > 0 && height > 0 ? { x, y, width, height } : undefined;
}

export function moveRegion(rect: Rect, dx: number, dy: number, bounds: Rect): Rect {
  return {
    ...rect,
    x: clamp(rect.x + dx, bounds.x, bounds.x + bounds.width - rect.width),
    y: clamp(rect.y + dy, bounds.y, bounds.y + bounds.height - rect.height),
  };
}

export function resizeRegion(rect: Rect, handle: string, point: Point, bounds: Rect): Rect {
  const right = rect.x + rect.width;
  const bottom = rect.y + rect.height;
  const x = handle.includes("w") ? clamp(point.x, bounds.x, right - 2) : rect.x;
  const y = handle.includes("n") ? clamp(point.y, bounds.y, bottom - 2) : rect.y;
  const nextRight = handle.includes("e") ? clamp(point.x, x + 2, bounds.x + bounds.width) : right;
  const nextBottom = handle.includes("s") ? clamp(point.y, y + 2, bounds.y + bounds.height) : bottom;
  return { x, y, width: nextRight - x, height: nextBottom - y };
}

export function paint(canvas: HTMLCanvasElement, image: HTMLImageElement, bounds: Rect, area: Rect, marks: Mark[]) {
  if (canvas.width !== area.width) canvas.width = area.width;
  if (canvas.height !== area.height) canvas.height = area.height;
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(image, area.x - bounds.x, area.y - bounds.y, area.width, area.height, 0, 0, area.width, area.height);
  const mosaic = document.createElement("canvas");
  for (const mark of marks) {
    const a = mark.points[0];
    const b = mark.points[mark.points.length - 1];
    const rect = rectangle(a, b);
    ctx.save();
    ctx.translate(-area.x, -area.y);
    ctx.strokeStyle = mark.color;
    ctx.fillStyle = mark.color;
    ctx.lineWidth = mark.width;
    ctx.lineJoin = "round";
    ctx.lineCap = "round";
    ctx.beginPath();
    switch (mark.tool) {
      case "mosaic": {
        const clipped = intersect(area, rect);
        if (!clipped) break;
        const x = clipped.x - area.x;
        const y = clipped.y - area.y;
        const block = Math.max(12, mark.width * 4);
        mosaic.width = Math.max(1, Math.ceil(clipped.width / block));
        mosaic.height = Math.max(1, Math.ceil(clipped.height / block));
        const sample = mosaic.getContext("2d");
        if (sample) {
          sample.imageSmoothingQuality = "high";
          sample.drawImage(canvas, x, y, clipped.width, clipped.height, 0, 0, mosaic.width, mosaic.height);
          ctx.imageSmoothingEnabled = false;
          ctx.drawImage(mosaic, clipped.x, clipped.y, clipped.width, clipped.height);
        }
        break;
      }
      case "cover":
        ctx.fillRect(rect.x, rect.y, rect.width, rect.height);
        break;
      case "rect":
        ctx.strokeRect(rect.x, rect.y, rect.width, rect.height);
        break;
      case "ellipse":
        ctx.ellipse(rect.x + rect.width / 2, rect.y + rect.height / 2, rect.width / 2, rect.height / 2, 0, 0, Math.PI * 2);
        ctx.stroke();
        break;
      case "arrow":
      case "line": {
        ctx.moveTo(a.x, a.y);
        ctx.lineTo(b.x, b.y);
        if (mark.tool === "arrow") {
          const angle = Math.atan2(b.y - a.y, b.x - a.x);
          const head = Math.min(mark.width * 4.5, Math.hypot(b.x - a.x, b.y - a.y) * 0.55);
          for (const offset of [-Math.PI / 6, Math.PI / 6]) {
            ctx.moveTo(b.x, b.y);
            ctx.lineTo(b.x - head * Math.cos(angle + offset), b.y - head * Math.sin(angle + offset));
          }
        }
        ctx.stroke();
        break;
      }
      case "pen":
      case "highlight":
        if (mark.tool === "highlight") {
          ctx.globalAlpha = 0.34;
          ctx.lineWidth = mark.width * 5;
        }
        ctx.moveTo(a.x, a.y);
        if (mark.points.length === 1) ctx.lineTo(a.x + 0.01, a.y);
        else for (const point of mark.points.slice(1)) ctx.lineTo(point.x, point.y);
        ctx.stroke();
        break;
      case "text":
        ctx.font = "600 " + Math.max(16, mark.width * 6) + "px 'Segoe UI', 'Microsoft YaHei', sans-serif";
        ctx.textBaseline = "top";
        ctx.fillText(mark.text ?? "", a.x, a.y);
        break;
      case "number": {
        const radius = Math.max(12, mark.width * 3.7);
        ctx.arc(a.x, a.y, radius, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = "#fff";
        ctx.lineWidth = 1.5;
        ctx.stroke();
        ctx.fillStyle = "#fff";
        ctx.font = "700 " + radius + "px 'Segoe UI', sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(mark.text ?? "", a.x, a.y + 1);
        break;
      }
    }
    ctx.restore();
  }
}

export async function canvasPng(canvas: HTMLCanvasElement): Promise<ArrayBuffer> {
  const blob = await new Promise<Blob>((resolve, reject) => {
    canvas.toBlob(value => value ? resolve(value) : reject(new Error("无法生成截图")), "image/png");
  });
  return blob.arrayBuffer();
}
