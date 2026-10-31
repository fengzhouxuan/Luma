export interface IconTarget {
  path: string;
  shortcut_path?: string | null;
}

export function iconKey(target: IconTarget): string {
  return JSON.stringify([target.path, target.shortcut_path ?? null]);
}

// 可见区域变化会替换待加载队列，已经发出的请求继续占用并发名额。
export class IconLoader {
  private generation = 0;
  private visible = new Map<string, IconTarget>();
  private active = new Set<string>();
  private completed = new Set<string>();
  private failures = new Map<string, number>();

  constructor(
    private readonly fetchIcon: (target: IconTarget) => Promise<string | null>,
    private readonly receiveIcon: (key: string, icon: string) => void,
    private readonly concurrency = 2,
    private readonly now: () => number = Date.now,
  ) {}

  reset(): void {
    this.generation++;
    this.visible.clear();
    this.completed.clear();
    this.failures.clear();
  }

  setVisible(targets: IconTarget[]): void {
    this.visible = new Map(targets.map((target) => [iconKey(target), target]));
    this.pump();
  }

  retry(): void {
    this.pump();
  }

  private pump(): void {
    for (const [key, target] of this.visible) {
      if (this.active.size >= this.concurrency) return;
      const failedAt = this.failures.get(key);
      if (this.active.has(key) || this.completed.has(key) ||
        (failedAt !== undefined && this.now() - failedAt < 30_000)) continue;
      const generation = this.generation;
      this.active.add(key);
      void Promise.resolve().then(() => this.fetchIcon(target)).then((icon) => {
        if (generation !== this.generation) return;
        if (icon) {
          this.completed.add(key);
          this.failures.delete(key);
          this.receiveIcon(key, icon);
        } else {
          this.failures.set(key, this.now());
        }
      }).catch(() => {
        if (generation === this.generation) this.failures.set(key, this.now());
      }).finally(() => {
        this.active.delete(key);
        this.pump();
      });
    }
  }
}
