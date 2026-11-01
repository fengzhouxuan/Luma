import { UiIcon } from "./UiIcon";
import type { AppEntry } from "./types";

export function EntryIcon({ app }: { app: AppEntry }) {
  if (app.kind && app.kind !== "app") return <span className={`entry-kind-icon kind-${app.kind}`} aria-hidden="true"><UiIcon name={app.kind === "folder" ? "folder" : app.kind === "web" ? "browser" : "code"}/></span>;
  return app.icon ? <img src={app.icon} alt="" draggable={false}/> : <span className="entry-fallback" aria-hidden="true">{app.name.charAt(0).toUpperCase()}</span>;
}
