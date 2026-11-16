import { create } from 'zustand';
import { api } from '@/lib/ipc';
import type { Settings, Theme, VideoInfo } from '@/types';

type Page = 'library' | 'plugins' | 'settings';

const defaultSettings: Settings = {
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

function systemPrefersDark(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function applyTheme(theme: Theme): void {
  if (typeof document === 'undefined') return;
  document.documentElement.classList.toggle(
    'dark',
    theme === 'dark' || (theme === 'system' && systemPrefersDark()),
  );
}

interface AppState {
  page: Page;
  settings: Settings;
  videos: VideoInfo[];
  selectedVideoId: string | null;
  loading: boolean;
  importing: boolean;
  error: string | null;
  notice: string | null;
  initialize: () => Promise<void>;
  applyTheme: () => void;
  setPage: (page: Page) => void;
  selectVideo: (id: string | null) => void;
  refreshVideos: () => Promise<void>;
  importPaths: (paths: string[]) => Promise<void>;
  deleteVideo: (id: string) => Promise<void>;
  updateSettings: (settings: Settings) => Promise<boolean>;
  clearNotice: () => void;
  clearError: () => void;
}

export const useAppStore = create<AppState>((set, get) => ({
  page: 'library',
  settings: defaultSettings,
  videos: [],
  selectedVideoId: null,
  loading: true,
  importing: false,
  error: null,
  notice: null,

  async initialize() {
    set({ loading: true, error: null });
    try {
      const [settings, videos] = await Promise.all([
        api.getSettings().catch(() => defaultSettings),
        api.getVideos().catch(() => []),
      ]);
      applyTheme(settings.theme);
      set({
        settings,
        videos,
        selectedVideoId: get().selectedVideoId ?? videos[0]?.id ?? null,
        loading: false,
      });
    } catch (error) {
      set({ loading: false, error: String(error) });
    }
  },

  applyTheme() {
    applyTheme(get().settings.theme);
  },

  setPage(page) {
    set({ page });
  },

  selectVideo(id) {
    set({ selectedVideoId: id });
  },

  async refreshVideos() {
    const videos = await api.getVideos();
    const selectedVideoId = get().selectedVideoId;
    set({
      videos,
      selectedVideoId: videos.some((v) => v.id === selectedVideoId)
        ? selectedVideoId
        : videos[0]?.id ?? null,
    });
  },

  async importPaths(paths) {
    if (paths.length === 0) return;
    set({ importing: true, error: null, notice: null });
    const failures: string[] = [];
    let imported = 0;
    let skipped = 0;

    try {
      for (const path of paths) {
        try {
          const result = await api.importVideo(path);
          if (!result.success) {
            failures.push(`${path}: ${result.error ?? '导入失败'}`);
          } else if (result.skipped) {
            skipped += 1;
          } else {
            imported += 1;
          }
        } catch (error) {
          failures.push(`${path}: ${String(error)}`);
        }
      }
      await get().refreshVideos();
      const parts = [`已导入 ${imported} 个视频`];
      if (skipped > 0) parts.push(`跳过 ${skipped} 个重复项`);
      if (failures.length > 0) parts.push(`${failures.length} 项失败`);
      set({
        importing: false,
        notice: parts.join('，'),
        error: failures.length > 0 ? failures.join('\n') : null,
      });
    } catch (error) {
      set({ importing: false, error: String(error) });
    }
  },

  async deleteVideo(id) {
    set({ error: null });
    try {
      await api.deleteVideo(id);
      await get().refreshVideos();
      set({ notice: '已从媒体库移除' });
    } catch (error) {
      set({ error: String(error) });
    }
  },

  async updateSettings(settings) {
    set({ error: null });
    try {
      await api.saveSettings(settings);
      applyTheme(settings.theme);
      set({ settings, notice: '设置已保存' });
      return true;
    } catch (error) {
      set({ error: String(error) });
      return false;
    }
  },

  clearNotice() {
    set({ notice: null });
  },

  clearError() {
    set({ error: null });
  },
}));