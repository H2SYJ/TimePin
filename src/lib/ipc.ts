import { convertFileSrc, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { listen as tauriListen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type {
  ExportMode,
  ImportResult,
  Mark,
  OrchestrationProgress,
  OrchestrationRequest,
  OrchestrationResult,
  PluginInfo,
  PluginResult,
  Settings,
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
    thumbnailQuality: 80,
    thumbnailMaxDimension: 320,
    maxConcurrentImports: 3,
    defaultExportMode: 'streamCopy',
    language: 'zh-CN',
    theme: 'system',
    hardwareAcceleration: true,
    customPluginDirs: [],
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
    if (!isTauri()) return;
    return tauriInvoke<void>('save_settings', { settings });
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

  // 插件系统 API (M2)
  async getPlugins(): Promise<PluginInfo[]> {
    if (!isTauri()) return [];
    return tauriInvoke<PluginInfo[]>('get_plugins');
  },

  async reloadPlugins(): Promise<PluginInfo[]> {
    if (!isTauri()) return [];
    return tauriInvoke<PluginInfo[]>('reload_plugins');
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
};