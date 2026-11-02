export interface FileEntry { id: string; name: string; path: string; is_dir: boolean; size: number | null }
export interface FileSearchResult {
  state: "ready" | "indexing" | "unavailable" | "error";
  message: string; version: string | null; backend: string | null;
  total: number; offset: number; items: FileEntry[];
}
export type FileFilter = "all" | "files" | "folders";
export function fileQuery(query: string, filter: FileFilter): string {
  const text = query.trim();
  // Empty queries probe connection status without enumerating a drive.
  if (!text || filter === "all") return text;
  return `${filter === "files" ? "file:" : "folder:"} <${text}>`;
}
export function fileSize(bytes: number | null): string {
  if (bytes === null) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}
export class SearchRevision {
  private revision = 0;
  next(): number { return ++this.revision; }
  isCurrent(value: number): boolean { return this.revision === value; }
}
