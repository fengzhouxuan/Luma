export type FileShortcut = "preview" | "next" | "previous" | "open" | "reveal" | "copy";
export function fileShortcut(event: { key: string; altKey: boolean; metaKey: boolean; ctrlKey: boolean; shiftKey: boolean; repeat: boolean; isComposing: boolean }, target: "search" | "result" | "other"): FileShortcut | null {
  if (event.isComposing || event.altKey || event.metaKey || target === "other") return null;
  if (event.ctrlKey) return !event.repeat && event.shiftKey && event.key.toLowerCase() === "c" ? "copy" : null;
  if (event.key === "ArrowDown") return "next";
  if (event.key === "ArrowUp") return "previous";
  if (event.repeat) return null;
  if (event.key === " " && !event.shiftKey && target === "result") return "preview";
  if (event.key === "Enter") return event.shiftKey ? "reveal" : "open";
  return null;
}
// Serialize IPC, skip superseded files, and let Close follow an in-flight startup.
export class PreviewQueue {
  private revision = 0;
  private tail: Promise<unknown> = Promise.resolve();
  constructor(private send: (id: string | null) => Promise<void>) {}
  submit(id: string | null): Promise<boolean> {
    const revision = ++this.revision;
    const job = this.tail.catch(() => {}).then(async () => {
      if (revision !== this.revision) return false;
      await this.send(id);
      return revision === this.revision;
    });
    this.tail = job;
    return job;
  }
}
export function previewError(reason: unknown): { missing: boolean; message: string } {
  if (reason && typeof reason === "object" && "message" in reason) return { missing: "code" in reason && reason.code === "unavailable", message: String(reason.message) };
  return { missing: false, message: String(reason) };
}
