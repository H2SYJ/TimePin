/** 所有跨 IPC 的类型与 Rust serde 结构一一对应（camelCase）。 */

export type ExportMode = 'streamCopy' | 'frameAccurate';
export type Theme = 'light' | 'dark' | 'system';

export interface WindowState {
  width: number;
  height: number;
  x: number | null;
  y: number | null;
  maximized: boolean;
  fullscreen: boolean;
}

export interface Settings {
  importDirectory: string | null;
  ffmpegPath: string | null;
  thumbnailQuality: number;
  thumbnailMaxDimension: number;
  maxConcurrentImports: number;
  defaultExportMode: ExportMode;
  language: string;
  theme: Theme;
  hardwareAcceleration: boolean;
  customPluginDirs: string[];
  recentFiles: string[];
  windowState: WindowState | null;
}

export interface VideoInfo {
  id: string;
  name: string;
  path: string;
  /** 导入时选择的原始文件；删除媒体库条目不会删除它。 */
  sourcePath: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  codec: string;
  fileSize: number;
  importedAt: string;
  thumbnailPath: string | null;
  sidecarTxtPath: string | null;
  sidecarCsvPath: string | null;
  sidecarXmlPath: string | null;
  tags: string[];
  metadata: Record<string, string>;
}

export interface VideoMetadata {
  id: string;
  duration: number;
  width: number;
  height: number;
  fps: number;
  codec: string;
  fileSize: number;
}

export interface ImportResult {
  success: boolean;
  video: VideoInfo | null;
  error: string | null;
  skipped: boolean;
}

export interface Mark {
  id: string;
  startTime: number;
  endTime: number | null;
  label: string;
  color: string | null;
  confidence: number | null;
  sourcePlugin: string | null;
  thumbnailPath: string | null;
  metadata: Record<string, string>;
}

export interface ImportProgressEvent {
  schemaVersion: '1.0.0';
  taskId: string;
  total: number;
  completed: number;
  currentFile: string | null;
  status: 'pending' | 'running' | 'completed' | 'failed';
}

export interface ExportProgressEvent {
  schemaVersion: '1.0.0';
  taskId: string;
  total: number;
  completed: number;
  currentSegment: string | null;
  status: 'pending' | 'running' | 'completed' | 'failed';
}