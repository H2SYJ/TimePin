import { convertFileSrc, invoke as tauriInvoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import type {
  ExportMode,
  ImportResult,
  Mark,
  Settings,
  VideoInfo,
  VideoMetadata,
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
};