export type EntryKind = "app" | "folder" | "web" | "script";
export interface AppEntry {
  kind?: EntryKind;
  id: string;
  name: string;
  path: string;
  shortcut_path?: string | null;
  arguments: string;
  working_dir?: string | null;
  source: string;
  source_path: string;
  icon: string | null;
  hidden_reason?: string | null;
  category: string;
}

export interface HotkeyConfig { mode: "combo"; modifiers: string[]; key: string }
export interface Usage { count: number; last_launch: number }
export interface ManualApp { kind?: EntryKind; id: string; name: string; path: string; arguments: string; working_dir: string | null; shortcut_path: string | null }
export interface LaunchOverride { arguments: string; working_dir: string | null }
export interface AutostartStatus { enabled: boolean; legacy: boolean; command: string | null }
export interface BackupPreview { json: string; manual_apps: number; aliases: number; categories: number; pinned: number; hidden: number; directories: number; missing: string[] }
export interface UpdateInfo { current: string; version: string; notes: string; published_at: string; available: boolean; can_install: boolean; message: string }
export interface Config {
  hotkey: HotkeyConfig;
  autostart: boolean;
  appearance: { opacity: number; corner_radius: number };
  show_all?: boolean;
  extra_dirs?: string[];
  custom_categories?: string[];
  app_categories?: Record<string, string>;
  pinned_apps?: string[];
  hidden_apps?: string[];
  usage?: Record<string, Usage>;
  manual_apps?: ManualApp[];
  aliases?: Record<string, string[]>;
  launch_overrides?: Record<string, LaunchOverride>;
  window_position?: {
    monitor_id: string;
    monitor_name: string;
    scale_factor: number;
    relative_x: number;
    relative_y: number;
  } | null;
}
export interface ScanReport {
  revision: number;
  full: boolean;
  rescanned: string[];
  duration_ms: number;
  time: number;
  app_count: number;
  watched_directories: number;
  watched_registry_keys: number;
  sources: { name: string; duration_ms: number; count: number; updated_at: number }[];
}
export interface ScanResult { apps: AppEntry[]; report: ScanReport }
export interface DiagnosticEvent { time: number; stage: string; path: string; app_id?: string | null; message: string }
export interface Diagnostics { version: string; executable: string; report: ScanReport; events: DiagnosticEvent[]; icons: { requests: number; completed: number; cache_hits: number; timeouts: number; average_queue_ms: number; max_queue_ms: number; average_extraction_ms: number } }
