import type { ReactNode } from "react";
type Name = "search" | "plus" | "refresh" | "settings" | "grid" | "game" | "office" | "code" | "media" | "chat" | "browser" | "other";
const paths: Record<Name, ReactNode> = {
  search: <><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 4.5 4.5"/></>,
  plus: <path d="M12 5v14M5 12h14"/>,
  refresh: <><path d="M20 10a8 8 0 1 0-2.3 7.7M20 4v6h-6"/></>,
  settings: <><path d="m9 3-.7 2.7-2.6.8-2 .3-1.4 2.4 1.9 2 .1 2.7-1.9 2 1.4 2.4 2.7-.6 2.4 1.4.7 2.7h2.8l.7-2.7 2.4-1.4 2.7.6 1.4-2.4-1.9-2 .1-2.7 1.9-2-1.4-2.4-2.7.6-2.4-1.4L13 3Z"/><circle cx="11" cy="12" r="3"/></>,
  grid: <><rect x="4" y="4" width="6" height="6" rx="1.5"/><rect x="14" y="4" width="6" height="6" rx="1.5"/><rect x="4" y="14" width="6" height="6" rx="1.5"/><rect x="14" y="14" width="6" height="6" rx="1.5"/></>,
  game: <><path d="M7 7h10c2 0 3 2 3.5 5l.5 4c.2 2-1.5 3-3 1.5L15 15H9l-3 2.5C4.5 19 2.8 18 3 16l.5-4C4 9 5 7 7 7Z"/><path d="M8 10v4M6 12h4M16 11h.01M18 13h.01"/></>,
  office: <><rect x="3" y="7" width="18" height="14" rx="3"/><path d="M8 7V4h8v3M3 12c6 3 12 3 18 0M12 12v4"/></>,
  code: <><path d="m8 6-6 6 6 6m8-12 6 6-6 6M14 4l-4 16"/></>,
  media: <><rect x="3" y="4" width="18" height="16" rx="3"/><path d="m10 8 6 4-6 4Z"/></>,
  chat: <path d="M5 4h14a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H9l-6 4V6a2 2 0 0 1 2-2Z"/>,
  browser: <><circle cx="12" cy="12" r="9"/><path d="m15 9-2 4-4 2 2-4Z"/></>,
  other: <><circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/></>,
};
export function UiIcon({ name }: { name: Name }) { return <svg className="ui-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">{paths[name]}</svg>; }
const categories: Record<string, Name> = { "全部":"grid", "游戏":"game", "办公":"office", "开发":"code", "媒体":"media", "社交":"chat", "浏览器":"browser", "其他":"other" };
export function CategoryIcon({ category }: { category: string }) { return <UiIcon name={categories[category] ?? "grid"}/>; }
