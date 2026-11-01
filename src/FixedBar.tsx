import { useEffect, type PointerEvent, type RefObject } from "react";
import { iconKey } from "./iconLoader";
import { adjacentBefore } from "./dock";
import { UiIcon } from "./UiIcon";
import type { AppDrag } from "./useAppDrag";
import type { AppEntry } from "./types";

interface Props {
  apps: AppEntry[]; listRef: RefObject<HTMLDivElement | null>; drag: AppDrag | null;
  onLaunch: (app: AppEntry) => void; onMenu: (app: AppEntry, x: number, y: number) => void;
  onPlace: (app: AppEntry, beforeId: string | null) => void;
  onDrag: (event: PointerEvent<HTMLElement>, app: AppEntry) => void;
}
export function FixedBar({ apps, listRef, drag, onLaunch, onMenu, onPlace, onDrag }: Props) {
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey && Math.abs(event.deltaY) > Math.abs(event.deltaX) && list.scrollWidth > list.clientWidth) {
        event.preventDefault(); list.scrollLeft += event.deltaY;
      }
    };
    list.addEventListener('wheel', wheel, { passive: false });
    return () => list.removeEventListener('wheel', wheel);
  }, [listRef]);
  return <section className={`fixed-bar${drag ? ' dragging' : ''}${drag?.overDock ? ' drop-active' : ''}`} aria-label="固定应用栏">
    <span className="fixed-label"><UiIcon name="star"/><span>固定</span></span>
    <div ref={listRef} className="fixed-list">
      {apps.length === 0 && <span className="fixed-empty">拖入应用，或右键应用选择「固定到应用栏」</span>}
      {apps.map(app => <div className="fixed-slot" key={app.id}>
        {drag?.overDock && drag.beforeId === app.id && <span className="fixed-insertion"/>}
        <button className={`fixed-app${drag?.app.id === app.id ? ' drag-source' : ''}`} data-dock-app={app.id} data-icon-key={iconKey(app)}
          title={app.name} aria-label={`${app.name}，固定应用`} aria-description="单击启动，右键管理，Alt 加左右方向键调整顺序"
          onClick={() => onLaunch(app)} onPointerDown={event => onDrag(event, app)}
          onContextMenu={event => { event.preventDefault(); onMenu(app, event.clientX, event.clientY); }}
          onKeyDown={event => {
            if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
              event.preventDefault(); event.stopPropagation();
              const rect = event.currentTarget.getBoundingClientRect(); onMenu(app, rect.left, rect.bottom); return;
            }
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
            event.preventDefault(); event.stopPropagation();
            if (event.altKey && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) {
              const before = adjacentBefore(apps, app.id, event.key === 'ArrowLeft' ? -1 : 1);
              if (before !== undefined) onPlace(app, before);
            } else {
              const buttons = [...listRef.current!.querySelectorAll<HTMLButtonElement>('[data-dock-app]')];
              const index = buttons.indexOf(event.currentTarget);
              const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : Math.max(0, Math.min(buttons.length - 1, index + (event.key === 'ArrowLeft' ? -1 : 1)));
              buttons[next]?.focus(); buttons[next]?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
            }
          }}>
          {app.icon ? <img src={app.icon} alt="" draggable={false}/> : <span className="fixed-fallback">{app.name.charAt(0).toUpperCase()}</span>}
        </button>
      </div>)}
      {drag?.overDock && !drag.beforeId && <span className="fixed-insertion at-end"/>}
    </div>
    {drag && <span className="fixed-drop-hint">松开固定</span>}
  </section>;
}
