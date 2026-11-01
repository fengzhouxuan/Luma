import type { ReactNode } from "react";
import { UiIcon } from "./UiIcon";

export function Dialog({ title, label, onClose, busy = false, className = "", children }: {
  title: string; label?: string; onClose: () => void; busy?: boolean; className?: string; children: ReactNode;
}) {
  return <div className="settings-overlay" onClick={() => !busy && onClose()}>
    <section className={`settings-panel ${className}`} role="dialog" aria-modal="true" aria-label={title}
      data-focus-scope onClick={event => event.stopPropagation()} tabIndex={-1}>
      <div className="settings-header"><span>{title}</span>
        <button className="settings-close" aria-label={label ?? "关闭面板"} disabled={busy} onClick={onClose}><UiIcon name="close"/></button>
      </div>
      <div className="panel-scroll">{children}</div>
    </section>
  </div>;
}
