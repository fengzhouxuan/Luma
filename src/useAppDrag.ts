import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import type { AppEntry } from "./types";
import { insertionBefore } from "./dock";

export interface AppDrag { app: AppEntry; x: number; y: number; overDock: boolean; beforeId: string | null; overCategory: string | null }

// Pointer dragging leaves Tauri's native file-drop support enabled on Windows.
export function useAppDrag(dock: RefObject<HTMLDivElement | null>, categories: RefObject<HTMLDivElement | null>, drop: (app: AppEntry, beforeId: string | null) => void, categorize: (app: AppEntry, category: string) => void) {
  const [drag, setDrag] = useState<AppDrag | null>(null);
  const pending = useRef<{ app: AppEntry; x: number; y: number; pointer: number; source: HTMLElement; active: boolean } | null>(null);
  const suppressed = useRef(false);
  const onDrop = useRef(drop);
  onDrop.current = drop;
  const onCategory = useRef(categorize);
  onCategory.current = categorize;
  useEffect(() => {
    const locate = (x: number, y: number, id: string) => {
      const node = dock.current;
      const rect = node?.getBoundingClientRect();
      const overDock = !!rect && x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
      if (node && overDock) {
        if (x < rect!.left + 28) node.scrollLeft -= 12;
        else if (x > rect!.right - 28) node.scrollLeft += 12;
      }
      const items = [...(node?.querySelectorAll<HTMLElement>('[data-dock-app]') ?? [])].map(item => {
        const bounds = item.getBoundingClientRect();
        return { id: item.dataset.dockApp!, center: bounds.left + bounds.width / 2 };
      });
      const sidebar = categories.current;
      const bounds = sidebar?.getBoundingClientRect();
      let overCategory: string | null = null;
      if (sidebar && bounds && x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom) {
        if (y < bounds.top + 24) sidebar.scrollTop -= 10;
        else if (y > bounds.bottom - 24) sidebar.scrollTop += 10;
        for (const item of sidebar.querySelectorAll<HTMLElement>('[data-drop-category]')) {
          const box = item.getBoundingClientRect();
          if (x >= box.left && x <= box.right && y >= box.top && y <= box.bottom) { overCategory = item.dataset.dropCategory!; break; }
        }
      }
      return { overDock, beforeId: insertionBefore(items, x, id), overCategory };
    };
    const cancel = () => {
      const item = pending.current;
      if (item?.active) {
        suppressed.current = true;
        try { if (item.source.hasPointerCapture(item.pointer)) item.source.releasePointerCapture(item.pointer); } catch { /* Source may have been removed by discovery. */ }
      }
      pending.current = null; setDrag(null);
    };
    const move = (event: PointerEvent) => {
      const item = pending.current;
      if (!item || event.pointerId !== item.pointer) return;
      if (!item.source.isConnected || !(event.buttons & 1)) { cancel(); return; }
      if (!item.active && Math.hypot(event.clientX - item.x, event.clientY - item.y) < 6) return;
      if (!item.active) {
        item.active = true;
        try { item.source.setPointerCapture(item.pointer); } catch { cancel(); return; }
      }
      event.preventDefault();
      setDrag({ app: item.app, x: event.clientX, y: event.clientY, ...locate(event.clientX, event.clientY, item.app.id) });
    };
    const up = (event: PointerEvent) => {
      const item = pending.current;
      if (!item || event.pointerId !== item.pointer) return;
      if (item.active) {
        event.preventDefault();
        const target = locate(event.clientX, event.clientY, item.app.id);
        if (target.overCategory !== null) onCategory.current(item.app, target.overCategory);
        else if (target.overDock) onDrop.current(item.app, target.beforeId);
      }
      cancel();
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && pending.current?.active) {
        event.preventDefault(); event.stopImmediatePropagation(); cancel();
      } else if (!pending.current?.active) suppressed.current = false;
    };
    const resetClick = () => { suppressed.current = false; };
    window.addEventListener('pointerdown', resetClick, true);
    window.addEventListener('pointermove', move, { passive: false });
    window.addEventListener('pointerup', up);
    window.addEventListener('pointercancel', cancel);
    window.addEventListener('blur', cancel);
    window.addEventListener('keydown', escape, true);
    return () => {
      window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up);
      window.removeEventListener('pointercancel', cancel); window.removeEventListener('blur', cancel);
      window.removeEventListener('keydown', escape, true);
      window.removeEventListener('pointerdown', resetClick, true);
    };
  }, [dock, categories]);
  return {
    drag,
    start: (event: ReactPointerEvent<HTMLElement>, app: AppEntry) => {
      if (event.button !== 0 || !event.isPrimary || event.pointerType === 'touch') return;
      pending.current = { app, x: event.clientX, y: event.clientY, pointer: event.pointerId, source: event.currentTarget, active: false };
    },
    consumeClick: () => {
      const consume = suppressed.current;
      suppressed.current = false;
      return consume;
    },
  };
}
