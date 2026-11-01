// Coalesce slider changes and serialize writes so an older request cannot win a race.
export class LatestWriter<T> {
  private pending: { value: T } | null = null;
  private running: Promise<void> | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  constructor(private write: (value: T) => Promise<void>, private onError: (error: unknown) => void, private delay = 120) {}
  set(value: T) {
    this.pending = { value };
    clearTimeout(this.timer);
    this.timer = setTimeout(() => { void this.flush(); }, this.delay);
  }
  flush(): Promise<void> {
    clearTimeout(this.timer);
    if (this.running) return this.running;
    this.running = (async () => {
      while (this.pending) {
        const { value } = this.pending;
        this.pending = null;
        try { await this.write(value); } catch (error) { this.onError(error); }
      }
    })().finally(() => {
      this.running = null;
      if (this.pending) void this.flush();
    });
    return this.running;
  }
}
