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

export interface PluginParameterSchema {
  type?: 'string' | 'number' | 'integer' | 'boolean';
  title?: string;
  description?: string;
  default?: string | number | boolean | null;
}

export type PluginParametersSchema = Record<string, PluginParameterSchema>;

export interface PluginInfo {
  name: string;
  version: string;
  description: string;
  author: string;
  runtime: string;
  capabilities: string[];
  parametersSchema: PluginParametersSchema;
  dir: string;
  status: 'ready' | 'invalid' | 'crashed' | 'running';
  statusDetail: string | null;
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

/** ========== M4：多插件编排 ========== */

export type OrchestrationMode = 'sequential' | 'parallel';

export interface OrchestrationRequest {
  pluginNames: string[];
  videos: VideoInfo[];
  perPluginParams?: Record<string, Record<string, unknown>>;
  defaultParams?: Record<string, unknown>;
  outputDir: string;
  mode?: OrchestrationMode;
  mergeEpsilonSecs?: number;
  maxConcurrency?: number;
}

export interface PluginRunOutcome {
  pluginName: string;
  videoId: string;
  // ok | error | crashed | skipped
  outcome: string;
  error?: string;
  marks: Mark[];
  taskId: string;
}

export interface VideoRunSummary {
  videoId: string;
  marks: Mark[];
  pluginResults: PluginRunOutcome[];
}

export interface OrchestrationResult {
  schemaVersion: string;
  batchId: string;
  videoSummaries: VideoRunSummary[];
  totalMarks: number;
  failedCount: number;
}

export interface OrchestrationProgress {
  schemaVersion: string;
  batchId: string;
  // started | running | video_done | finished | cancelled
  stage: string;
  completed: number;
  total: number;
  currentVideoId?: string;
  currentPluginName?: string;
  lastOutcome?: PluginRunOutcome;
}

/** ========== 识别服务类型（M3） ========== */

export interface NormalizedRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface TemplateMatchResult {
  rect: NormalizedRect;
  confidence: number;
  scale: number;
  templateName: string;
  timestamp: number;
}

export interface OcrResult {
  rect: NormalizedRect;
  text: string;
  confidence: number;
  timestamp: number;
}

export interface DetectionResult {
  rect: NormalizedRect;
  label: string;
  confidence: number;
  classId: number;
  timestamp: number;
}

export interface PreviewFrame {
  frameIndex: number;
  timestamp: number;
  imageBase64: string;
  width: number;
  height: number;
}

export interface RecognitionTemplateMatchParams {
  taskId: string;
  videoPath: string;
  templateName: string;
  threshold: number;
  scales?: number[];
  roi?: NormalizedRect;
  frameTimestamps?: number[];
  maxFrames: number;
}

export interface RecognitionOcrParams {
  taskId: string;
  videoPath: string;
  roi?: NormalizedRect;
  frameTimestamps?: number[];
  maxFrames: number;
  lang?: string;
}

export interface RecognitionDetectParams {
  taskId: string;
  videoPath: string;
  roi?: NormalizedRect;
  frameTimestamps?: number[];
  maxFrames: number;
}

export interface RecognitionPreviewParams {
  taskId: string;
  videoPath: string;
  timestamp: number;
  templateMatches?: TemplateMatchResult[];
  ocrResults?: OcrResult[];
  detections?: DetectionResult[];
  rois?: NormalizedRect[];
  quality?: number;
}

export interface RecognitionRoiSaveParams {
  name: string;
  rect: NormalizedRect;
  metadata?: Record<string, unknown>;
}

export interface RecognitionRoiLoadParams {
  name: string;
}

export interface RecognitionConfigureParams {
  ocr?: Record<string, unknown>;
  detector?: Record<string, unknown>;
  templates?: Record<string, unknown>;
}