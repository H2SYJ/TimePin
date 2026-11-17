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

/** ========== 插件系统类型（M2） ========== */

export interface PluginCapability {
  // videoFrames | audioSamples | metadata | gpu | network | fileWrite
  type: string;
}

export interface PluginManifest {
  protocolVersion: string;
  name: string;
  version: string;
  description: string;
  author: string;
  entry: string;
  runtime: 'python' | 'rust' | 'node' | 'binary';
  capabilities: string[];
  parametersSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
}

export interface PluginInfo {
  name: string;
  version: string;
  description: string;
  author: string;
  runtime: string;
  capabilities: string[];
  parametersSchema: Record<string, PluginParamSchema>;
  dir: string;
  status: 'ready' | 'invalid' | 'crashed' | 'running';
  statusDetail: string | null;
}

/** 插件参数 JSON Schema 定义 */
export interface PluginParamSchema {
  type?: 'string' | 'number' | 'integer' | 'boolean' | 'array';
  title?: string;
  description?: string;
  default?: unknown;
  enum?: unknown[];
  items?: { enum?: unknown[] };
  minimum?: number;
  maximum?: number;
}

/** 推断出的参数控件类型 */
export type ParamKind = 'text' | 'number' | 'boolean' | 'select' | 'multiselect';

/** 根据 JSON Schema 推断控件类型 */
export function inferParamKind(schema: PluginParamSchema): ParamKind {
  if (Array.isArray(schema.enum) && schema.enum.length > 0) return 'select';
  if (schema.type === 'array' && Array.isArray(schema.items?.enum)) return 'multiselect';
  if (schema.type === 'boolean') return 'boolean';
  if (schema.type === 'number' || schema.type === 'integer') return 'number';
  return 'text';
}

export interface PluginRunParams {
  taskId: string;
  video: {
    id: string;
    path: string;
    duration: number;
    width: number;
    height: number;
    fps: number;
    metadata: Record<string, string>;
  };
  params: Record<string, unknown>;
  outputDir: string;
}

export interface PluginResult {
  protocolVersion: string;
  pluginName: string;
  marks: Mark[];
  logs: string[];
  evidencePaths: string[];
}

export interface PluginProgressEvent {
  schemaVersion: string;
  pluginName: string;
  taskId: string;
  progress: number;
  message: string | null;
}

export interface PluginStatusEvent {
  schemaVersion: string;
  pluginName: string;
  // discovered | validated | started | ready | running | stopped | crashed
  status: string;
  detail?: string;
}

export interface PluginRunFinished {
  schemaVersion: string;
  taskId: string;
  pluginName: string;
  videoId: string;
  // ok | error | cancelled | crashed
  outcome: string;
  error?: string;
  marksCount: number;
}