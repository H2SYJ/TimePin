/** 所有跨 IPC 的类型与 Rust serde 结构一一对应（camelCase）。 */

export type {
  UpdateInfo,
  UpdatePhase,
  UpdateProgress,
  UpdaterConfig,
} from './updater';

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
  ffprobePath: string | null;
  thumbnailQuality: number;
  thumbnailMaxDimension: number;
  maxConcurrentImports: number;
  defaultExportMode: ExportMode;
  language: string;
  theme: Theme;
  hardwareAcceleration: boolean;
  customPluginDirs: string[];
  /** 用户显式配置的 Python 解释器绝对路径（优先于内嵌运行时） */
  pythonPath: string | null;
  /** 是否允许回退到系统 PATH 中的 python/python3 */
  useSystemPython: boolean;
  recentFiles: string[];
  windowState: WindowState | null;
}

export interface AdvancedSettingsInfo {
  schemaVersion: string;
  defaultPluginDirectory: string;
  installDirectory: string;
}

export interface DependencyStatus {
  id: 'ffmpeg' | 'ffprobe';
  available: boolean;
  path: string | null;
  version: string | null;
  error: string | null;
  minimumVersion: string;
}

export interface DependencyReport {
  schemaVersion: string;
  dependencies: DependencyStatus[];
}

export interface DependencyInstallProgress {
  schemaVersion: string;
  taskId: string;
  stage: 'download' | 'extract' | 'validate' | 'completed' | 'failed';
  progress: number;
  message: string;
  error: string | null;
}

export interface InstalledDependencies {
  schemaVersion: string;
  ffmpegPath: string;
  ffprobePath: string;
  report: DependencyReport;
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

/**
 * 可本地化文本：普通字符串，或 `locale -> text` 映射。
 * 解析顺序见 `resolveLocalizedText`：settings.language -> "en-US" -> 映射首个条目 -> ""。
 */
export type LocalizedText = string | Record<string, string>;

export interface PluginManifest {
  protocolVersion: string;
  name: string;
  version: string;
  description: LocalizedText;
  author: string;
  entry: string;
  runtime: 'python' | 'rust' | 'node' | 'binary';
  capabilities: string[];
  parametersSchema: Record<string, unknown>;
  outputSchema: Record<string, unknown>;
  /** Python 版本约束（仅 runtime=python），如 `">=3.10,<3.13"`、`"3.11.*"` */
  python?: string | null;
  /** pip 依赖（仅 runtime=python），自动建 venv 安装 */
  dependencies?: string[];
}

export interface PluginParameterSchema {
  type?: 'string' | 'number' | 'integer' | 'boolean';
  title?: LocalizedText;
  description?: LocalizedText;
  default?: string | number | boolean | null;
}

export type PluginParametersSchema = Record<string, PluginParameterSchema>;

export interface PluginEnvStatus {
  /** absent=未建 / ready=可用 / failed=建置失败 / creating-venv / installing */
  stage: 'absent' | 'ready' | 'failed' | 'creating-venv' | 'installing';
  venvDir: string;
  pythonVersion: string | null;
  error: string | null;
}

export interface PluginInfo {
  name: string;
  version: string;
  description: LocalizedText;
  author: string;
  runtime: string;
  capabilities: string[];
  parametersSchema: PluginParametersSchema;
  dir: string;
  status: 'ready' | 'invalid' | 'crashed' | 'running';
  statusDetail: string | null;
  /** Python 插件的 venv 状态；非 Python 插件为 null */
  pythonEnvironment?: PluginEnvStatus | null;
}

/** `plugin://env-progress` 事件的 payload：venv 建置进度 */
export interface PluginEnvProgress {
  schemaVersion: string;
  pluginName: string;
  stage: 'creating-venv' | 'installing' | 'ready' | 'failed';
  message: string;
  error: string | null;
}

/** `rebuild_plugin_environments` 命令的单插件结果 */
export interface EnvRebuildResult {
  name: string;
  venvDir: string | null;
  error: string | null;
}

/** 插件参数 JSON Schema 定义 */
export interface PluginParamSchema {
  type?: 'string' | 'number' | 'integer' | 'boolean' | 'array';
  title?: LocalizedText;
  description?: LocalizedText;
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

/** 应用数据目录内按视频持久化的标注文档（时间单位：秒）。 */
export interface MarksDocument {
  schemaVersion: '1.0.0';
  videoId: string;
  marks: Mark[];
}

/** ========== GitHub 插件发现类型 ========== */

export interface RemotePluginRepository {
  name: string;
  fullName: string;
  description: string | null;
  htmlUrl: string;
  cloneUrl: string;
  stargazersCount: number;
  forksCount: number;
  updatedAt: string;
  language: string | null;
  topics: string[];
  owner: {
    login: string;
    avatarUrl: string;
    htmlUrl: string;
  };
  defaultBranch: string;
}

export type PluginVerification = 'verified' | 'community' | 'unverified';

export interface DiscoveredPlugin {
  repository: RemotePluginRepository;
  /** 复用同文件上方 PluginManifest 定义 */
  manifest: PluginManifest | null;
  manifestError: string | null;
  isInstalled: boolean;
  localPath: string | null;
  verification: PluginVerification;
}

/** GitHub 速率限制信息；所有时间戳均为本地校准后的 epoch 毫秒。 */
export interface GitHubRateInfo {
  limit: number | null;
  remaining: number | null;
  resetAt: number | null;
  resource: string | null;
}

export type GitHubApiErrorCode =
  | 'AUTH_INVALID'
  | 'PRIMARY_RATE_LIMIT'
  | 'SECONDARY_RATE_LIMIT'
  | 'RATE_LIMIT_UNKNOWN'
  | 'RATE_WAIT'
  | 'NOT_FOUND_OR_PRIVATE'
  | 'FORBIDDEN'
  | 'CLIENT_ERROR'
  | 'SERVER_ERROR'
  | 'NETWORK_ERROR';

export type GitHubApiErrorAction = 'WAIT' | 'CONFIGURE_TOKEN' | 'REAUTHENTICATE' | 'NONE';

/** 所有 GitHub IPC 失败均以该对象 reject（不是 JSON 字符串）。 */
export interface GitHubApiError {
  schemaVersion: '1.0.0';
  code: GitHubApiErrorCode;
  message: string;
  retryAt: number | null;
  action: GitHubApiErrorAction;
  rateLimit: GitHubRateInfo | null;
}

/** GitHub 登录会话；token 本体永远不返回前端。 */
export interface GitHubSession {
  schemaVersion: '1.0.0';
  identity: 'ANONYMOUS' | 'AUTHENTICATED' | 'TOKEN_INVALID';
  remembered: boolean;
  /** 凭据库写入失败等警告；非 null 时 token 仅保存在内存 */
  storageWarning: string | null;
  rateLimit: GitHubRateInfo | null;
  /** 各资源桶的独立快照；旧后端可省略，回退到 rateLimit。 */
  rateLimits?: GitHubRateInfo[];
}

export interface DiscoverPluginsResult {
  schemaVersion: '1.0.0';
  plugins: DiscoveredPlugin[];
  totalCount: number;
  rateLimit: GitHubRateInfo | null;
  rateLimits?: GitHubRateInfo[];
}

export interface InstallPluginParams {
  owner: string;
  repo: string;
  branch?: string;
}
