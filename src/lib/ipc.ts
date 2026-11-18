import { convertFileSrc, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type {
  AdvancedSettingsInfo,
  DependencyInstallProgress,
  DependencyReport,
  InstalledDependencies,
  ExportMode,
  ImportResult,
  Mark,
  MarksDocument,
  OrchestrationProgress,
  OrchestrationRequest,
  OrchestrationResult,
  EnvRebuildResult,
  PluginEnvProgress,
  PluginInfo,
  PluginImportSource,
  PluginImportResult,
  PluginResult,
  Settings,
  UpdaterConfig,
  VideoInfo,
  VideoMetadata,
  // M3 识别类型
  NormalizedRect,
  TemplateMatchResult,
  OcrResult,
  DetectionResult,
  PreviewFrame,
  RecognitionTemplateMatchParams,
  RecognitionOcrParams,
  RecognitionDetectParams,
  RecognitionPreviewParams,
  RecognitionRoiSaveParams,
  RecognitionRoiLoadParams,
  RecognitionConfigureParams,
  // GitHub 插件发现类型
  DiscoverPluginsResult,
  InstallPluginParams,
  GitHubSession,
} from '@/types';

export function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

/** 将本地路径转换为可被 WebView 读取的 asset URL。 */
export function fileSrc(path: string): string {
  if (!path) return '';
  return isTauri() ? convertFileSrc(path) : path;
}

function browserSettings(): Settings {
  return {
    importDirectory: null,
    ffmpegPath: null,
    ffprobePath: null,
    thumbnailQuality: 80,
    thumbnailMaxDimension: 320,
    maxConcurrentImports: 3,
    defaultExportMode: 'streamCopy',
    language: 'zh-CN',
    theme: 'system',
    hardwareAcceleration: true,
    customPluginDirs: [],
    pythonPath: null,
    useSystemPython: false,
    recentFiles: [],
    windowState: null,
  };
}

/** IPC wrapper: 桌面环境走 Tauri，普通浏览器构建提供安全的只读降级。 */
export const api = {
  async getSettings(): Promise<Settings> {
    if (!isTauri()) return browserSettings();
    return tauriInvoke<Settings>('get_settings');
  },

  async saveSettings(settings: Settings): Promise<void> {
    if (!isTauri()) throw new Error('保存设置需要在 TimePin 桌面应用中运行');
    return tauriInvoke<void>('save_settings', { settings });
  },

  async getAdvancedSettingsInfo(): Promise<AdvancedSettingsInfo> {
    if (!isTauri()) throw new Error('高级设置需要在 TimePin 桌面应用中运行');
    return tauriInvoke<AdvancedSettingsInfo>('get_advanced_settings_info');
  },

  async validateDependencies(settings: Settings): Promise<DependencyReport> {
    if (!isTauri()) throw new Error('依赖检测需要在 TimePin 桌面应用中运行');
    return tauriInvoke<DependencyReport>('validate_dependencies', { settings });
  },

  async openPluginDirectory(index: number | null): Promise<void> {
    if (!isTauri()) throw new Error('打开插件目录需要在 TimePin 桌面应用中运行');
    return tauriInvoke<void>('open_plugin_directory', { index });
  },

  async pickDirectory(title = '选择插件搜索目录'): Promise<string | null> {
    if (!isTauri()) throw new Error('目录选择需要在 TimePin 桌面应用中运行');
    const selected = await open({ directory: true, multiple: false, title });
    return Array.isArray(selected) ? selected[0] ?? null : selected;
  },

  async pickExecutable(tool: 'ffmpeg' | 'ffprobe'): Promise<string | null> {
    if (!isTauri()) throw new Error('工具选择需要在 TimePin 桌面应用中运行');
    const selected = await open({ multiple: false, title: `选择 ${tool} 可执行文件` });
    return Array.isArray(selected) ? selected[0] ?? null : selected;
  },

  /** 选择 Python 解释器（python.exe / python3）。返回绝对路径或 null。 */
  async pickPythonExecutable(): Promise<string | null> {
    if (!isTauri()) throw new Error('选择 Python 解释器需要在 TimePin 桌面应用中运行');
    const selected = await open({
      multiple: false,
      title: '选择 Python 解释器',
      filters: [{ name: 'Python', extensions: ['exe', 'bat', 'sh'] }],
    });
    return Array.isArray(selected) ? selected[0] ?? null : selected;
  },

  async installDependencies(taskId: string): Promise<InstalledDependencies> {
    if (!isTauri()) throw new Error('依赖安装需要在 TimePin 桌面应用中运行');
    return tauriInvoke<InstalledDependencies>('install_dependencies', { taskId });
  },

  async onDependencyInstallProgress(
    handler: (event: DependencyInstallProgress) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) throw new Error('依赖安装监听需要在 TimePin 桌面应用中运行');
    return tauriListen<DependencyInstallProgress>('dependency://install-progress', (event) => handler(event.payload));
  },

  async getVideos(): Promise<VideoInfo[]> {
    if (!isTauri()) return [];
    return tauriInvoke<VideoInfo[]>('get_videos');
  },

  async importVideo(path: string): Promise<ImportResult> {
    if (!isTauri()) {
      throw new Error('视频导入需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<ImportResult>('import_video', { sourcePath: path });
  },

  async deleteVideo(id: string): Promise<void> {
    if (!isTauri()) return;
    return tauriInvoke<void>('delete_video', { id });
  },

  async getVideoMetadata(path: string): Promise<VideoMetadata> {
    if (!isTauri()) {
      throw new Error('元数据读取需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<VideoMetadata>('get_video_metadata', { path });
  },

  /** 检查发布配置是否启用了应用内更新。 */
  async getUpdaterConfig(): Promise<UpdaterConfig> {
    if (!isTauri()) return { schemaVersion: '1.0.0', configured: false };
    return tauriInvoke<UpdaterConfig>('get_updater_config');
  },

  async generateThumbnail(videoPath: string, videoId: string): Promise<string> {
    if (!isTauri()) {
      throw new Error('缩略图生成需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<string>('generate_thumbnail', { videoPath, videoId });
  },

  async generateTxtChapters(marks: Mark[], videoDuration: number): Promise<string> {
    if (!isTauri()) return '';
    return tauriInvoke<string>('generate_txt_chapters', { marks, videoDuration });
  },

  async generateCsvMarkers(marks: Mark[], videoDuration: number): Promise<string> {
    if (!isTauri()) return '';
    return tauriInvoke<string>('generate_csv_markers', { marks, videoDuration });
  },

  async exportSegments(
    videoPath: string,
    marks: Mark[],
    outputDir: string,
    exportMode: ExportMode,
  ): Promise<string[]> {
    if (!isTauri()) return [];
    return tauriInvoke<string[]>('export_segments', {
      videoPath,
      marks,
      outputDir,
      exportMode,
    });
  },

  async getMarks(videoId: string): Promise<MarksDocument> {
    if (!isTauri()) return { schemaVersion: '1.0.0', videoId, marks: [] };
    return tauriInvoke<MarksDocument>('get_marks', { videoId });
  },

  async saveMarks(videoId: string, marks: Mark[]): Promise<MarksDocument> {
    if (!isTauri()) return { schemaVersion: '1.0.0', videoId, marks };
    return tauriInvoke<MarksDocument>('save_marks', { videoId, marks });
  },

  // 插件系统 API (M2)
  async getPlugins(): Promise<PluginInfo[]> {
    if (!isTauri()) return [];
    return tauriInvoke<PluginInfo[]>('get_plugins');
  },

  async reloadPlugins(): Promise<PluginInfo[]> {
    if (!isTauri()) return [];
    return tauriInvoke<PluginInfo[]>('reload_plugins');
  },

  async importPlugin(source: PluginImportSource): Promise<PluginImportResult> {
    if (!isTauri()) throw new Error('插件导入需要在 TimePin 桌面应用中运行');
    return tauriInvoke<PluginImportResult>('import_plugin', { source, schemaVersion: '1.0' });
  },

  async pickPluginPackage(): Promise<string | null> {
    if (!isTauri()) throw new Error('文件选择需要在 TimePin 桌面应用中运行');
    const selected = await open({
      multiple: false,
      title: '选择插件包',
      filters: [{ name: 'TimePin 插件包', extensions: ['tppkg', 'zip'] }],
    });
    if (Array.isArray(selected)) {
      if (selected.length > 1) throw new Error('一次只能导入一个插件包');
      return selected[0] ?? null;
    }
    return selected;
  },

  /** 重建所有 Python 插件的虚拟环境。返回每个插件的建置结果。 */
  async rebuildPluginEnvironments(): Promise<EnvRebuildResult[]> {
    if (!isTauri()) throw new Error('重建插件环境需要在 TimePin 桌面应用中运行');
    return tauriInvoke<EnvRebuildResult[]>('rebuild_plugin_environments');
  },

  /** 监听 Python 环境建置进度（venv create / pip install / ready / failed） */
  async onPluginEnvProgress(
    handler: (event: PluginEnvProgress) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('plugin://env-progress', (e: any) => handler(e.payload));
  },

  async runPlugin(
    pluginName: string,
    video: VideoInfo,
    params: Record<string, unknown>,
    outputDir: string,
  ): Promise<PluginResult> {
    if (!isTauri()) {
      throw new Error('插件执行需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<PluginResult>('run_plugin', {
      pluginName,
      video,
      params,
      outputDir,
    });
  },

  async stopPlugin(pluginName: string): Promise<void> {
    if (!isTauri()) return;
    return tauriInvoke<void>('stop_plugin', { pluginName });
  },

  async cancelPluginRun(pluginName: string, taskId: string): Promise<void> {
    if (!isTauri()) return;
    return tauriInvoke<void>('cancel_plugin_run', { pluginName, taskId });
  },

  // M4：多插件编排——批量在多个视频上执行多个插件，单点失败隔离 + 结果合并去重
  async runPlugins(request: OrchestrationRequest): Promise<OrchestrationResult> {
    if (!isTauri()) {
      throw new Error('多插件编排需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<OrchestrationResult>('run_plugins', { request });
  },

  async onPluginOrchestration(
    handler: (event: OrchestrationProgress) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('plugin://orchestration', (e: any) => handler(e.payload));
  },

  // 监听插件事件（返回取消监听函数）
  async onPluginProgress(
    handler: (event: { pluginName: string; taskId: string; progress: number; message: string | null }) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('plugin://progress', (e: any) => handler(e.payload));
  },

  async onPluginStatus(
    handler: (event: { pluginName: string; status: string; detail?: string }) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('plugin://status', (e: any) => handler(e.payload));
  },

  async onPluginLog(
    handler: (event: { pluginName: string; taskId: string; level: string; message: string }) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('plugin://log', (e: any) => handler(e.payload));
  },

  async onPluginResult(
    handler: (event: { taskId: string; pluginName: string; videoId: string; outcome: string; error?: string; marksCount: number }) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('plugin://result', (e: any) => handler(e.payload));
  },

  async pickVideoFiles(): Promise<string[]> {
    if (!isTauri()) {
      throw new Error('文件选择需要在 TimePin 桌面应用中运行');
    }
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: '视频文件',
          extensions: ['mp4', 'mkv', 'mov', 'avi', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts'],
        },
      ],
    });
    if (!selected) return [];
    return Array.isArray(selected) ? selected : [selected];
  },

  // 识别服务 API (M3)
  async recognitionTemplateMatch(params: RecognitionTemplateMatchParams): Promise<{ matches: TemplateMatchResult[] }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_template_match', { params });
  },

  async recognitionOcr(params: RecognitionOcrParams): Promise<{ results: OcrResult[] }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_ocr', { params });
  },

  async recognitionDetect(params: RecognitionDetectParams): Promise<{ results: DetectionResult[] }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_detect', { params });
  },

  async recognitionPreview(params: RecognitionPreviewParams): Promise<PreviewFrame> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_preview', { params });
  },

  async recognitionRoiSave(params: RecognitionRoiSaveParams): Promise<{ path: string }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_roi_save', { params });
  },

  async recognitionRoiLoad(params: RecognitionRoiLoadParams): Promise<{ rect: NormalizedRect }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_roi_load', { params });
  },

  async recognitionRoiList(): Promise<{ rois: string[] }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_roi_list', {});
  },

  async recognitionConfigure(params: RecognitionConfigureParams): Promise<{ ok: boolean }> {
    if (!isTauri()) throw new Error('识别服务需要在 TimePin 桌面应用中运行');
    return tauriInvoke('recognition_configure', { params });
  },

  // 监听识别进度事件
  async onRecognitionProgress(
    handler: (event: { taskId: string; progress: number; message?: string }) => void,
  ): Promise<UnlistenFn> {
    if (!isTauri()) return () => {};
    return tauriListen('recognition://progress', (e: any) => handler(e.payload));
  },

  // GitHub 插件发现 API（token 由 Rust 端托管，前端不持有、不传递）
  async discoverGitHubPlugins(
    page?: number,
    perPage?: number,
  ): Promise<DiscoverPluginsResult> {
    if (!isTauri()) {
      return { schemaVersion: '1.0.0', plugins: [], totalCount: 0, rateLimit: null };
    }
    return tauriInvoke<DiscoverPluginsResult>('discover_github_plugins', { page, perPage });
  },

  /** 安装成功时返回实际插件目录路径，供发现结果合并 localPath。 */
  async installGitHubPlugin(params: InstallPluginParams): Promise<string> {
    if (!isTauri()) {
      throw new Error('插件安装需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<string>('install_github_plugin', {
      owner: params.owner,
      repo: params.repo,
      branch: params.branch,
    });
  },

  /** 查询当前 GitHub 会话（匿名/已认证/token 失效 + 速率限制快照）。 */
  async getGitHubSession(): Promise<GitHubSession> {
    if (!isTauri()) {
      return {
        schemaVersion: '1.0.0',
        identity: 'ANONYMOUS',
        remembered: false,
        storageWarning: null,
        rateLimit: null,
      };
    }
    return tauriInvoke<GitHubSession>('get_github_session');
  },

  /**
   * 提交候选 token：后端必须先以 `/user` 200 验证通过才会应用。
   * remember=false 时仅保存在内存；remember=true 写入系统凭据库。
   * 失败以 GitHubApiError 对象 reject。
   */
  async configureGitHubToken(candidate: string, remember: boolean): Promise<GitHubSession> {
    if (!isTauri()) {
      throw new Error('Token 配置需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<GitHubSession>('configure_github_token', { candidate, remember });
  },

  /** 清除已保存的 GitHub token（内存与凭据库）。 */
  async clearGitHubToken(): Promise<GitHubSession> {
    if (!isTauri()) {
      throw new Error('Token 清除需要在 TimePin 桌面应用中运行');
    }
    return tauriInvoke<GitHubSession>('clear_github_token');
  },
};