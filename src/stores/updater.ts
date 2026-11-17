import { getVersion } from '@tauri-apps/api/app';
import { check, type DownloadEvent, type Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { create } from 'zustand';
import { api, isTauri } from '@/lib/ipc';
import type { UpdateInfo, UpdatePhase, UpdateProgress } from '@/types/updater';

const UPDATE_TIMEOUT_MS = 15_000;

type WorkKind = 'import' | 'export' | 'plugin' | 'recognition';

export type { WorkKind };

let nextTaskId = 0;

/** A running task token; callers pass the returned symbol to endWork. */
export interface WorkToken {
  kind: WorkKind;
  id: number;
}

/** Windows 安装器启动后当前进程即退出，install() 通常不会返回。 */
export function isWindows(): boolean {
  if (typeof navigator === 'undefined') return false;
  const ua = navigator.userAgent;
  return (
    ua.includes('Windows') ||
    // @ts-expect-error userAgentData 仅 Chromium 提供
    (navigator.userAgentData?.platform as string | undefined) === 'Windows'
  );
}

interface UpdateErrorState {
  message: string;
  action: 'check' | 'download' | 'install' | 'config';
}

interface UpdateState {
  phase: UpdatePhase;
  currentVersion: string;
  /** 检查成功时的原始错误，重试下载/安装时保留。 */
  available: UpdateInfo | null;
  progress: UpdateProgress;
  error: UpdateErrorState | null;
  busyWorkKinds: WorkKind[];
  /** Per-task tokens for concurrent same-kind runs. */
  workTokens: WorkToken[];
  /** 当前持有的 Tauri updater 资源；不进入 React 状态树。 */
  checkForUpdate: () => Promise<void>;
  requestInstallConfirmation: () => void;
  cancelInstallConfirmation: () => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  restartNow: () => Promise<void>;
  resetToAvailable: () => Promise<void>;
  clearError: () => void;
  /** Begin a task; returns a token passed to endWork when done. */
  beginWork: (kind: WorkKind) => WorkToken;
  endWork: (token: WorkToken) => void;
}

let updateResource: Update | null = null;

async function closeUpdateResource() {
  const current = updateResource;
  updateResource = null;
  if (current) {
    await current.close().catch(() => undefined);
  }
}

function normalizeError(error: unknown, action: UpdateErrorState['action']): UpdateErrorState {
  const raw = error instanceof Error ? error.message : String(error);
  const message = raw.trim() || '发生未知错误';
  const lower = message.toLowerCase();
  if (lower.includes('signature') || lower.includes('签名校验')) {
    return {
      action,
      message: `更新包签名校验失败，已取消安装。请稍后重试或等待新的发布。原始信息：${message}`,
    };
  }
  if (lower.includes('network') || lower.includes('fetch') || lower.includes('timeout') || lower.includes('连接')) {
    return {
      action,
      message: `无法连接更新服务器，请检查网络后重试。原始信息：${message}`,
    };
  }
  if (lower.includes('public key') || lower.includes('pubkey') || lower.includes('updater')) {
    return {
      action,
      message: `应用更新配置无效，请联系维护者检查发布配置。原始信息：${message}`,
    };
  }
  return { action, message };
}

function extractUpdateInfo(update: Update): UpdateInfo {
  return {
    currentVersion: update.currentVersion,
    version: update.version,
    date: update.date,
    body: update.body,
  };
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  phase: 'idle',
  currentVersion: '',
  available: null,
  progress: { downloadedBytes: 0, totalBytes: null },
  error: null,
  busyWorkKinds: [],
  workTokens: [],

  async checkForUpdate() {
    if (get().phase === 'checking' || get().phase === 'downloading' || get().phase === 'installing') {
      return;
    }
    if (!isTauri()) {
      set({ phase: 'desktopOnly', error: null, available: null });
      return;
    }

    set({ phase: 'checking', error: null });
    try {
      const [currentVersion, config] = await Promise.all([
        getVersion(),
        api.getUpdaterConfig(),
      ]);
      set({ currentVersion });

      if (!config.configured) {
        await closeUpdateResource();
        set({
          phase: 'notConfigured',
          available: null,
          error: {
            action: 'config',
            message: '此构建未配置应用内更新。请在发布环境设置 TAURI_UPDATER_PUBLIC_KEY 后重试，或联系维护者检查构建配置。',
          },
        });
        return;
      }

      const update = await check({ timeout: UPDATE_TIMEOUT_MS });
      await closeUpdateResource();
      if (!update) {
        set({ phase: 'upToDate', available: null, progress: { downloadedBytes: 0, totalBytes: null } });
        return;
      }

      updateResource = update;
      set({
        phase: 'available',
        available: extractUpdateInfo(update),
        progress: { downloadedBytes: 0, totalBytes: null },
      });
    } catch (error) {
      await closeUpdateResource();
      set({ phase: 'error', available: null, error: normalizeError(error, 'check') });
    }
  },

  requestInstallConfirmation() {
    const { phase, available } = get();
    if (!available || phase !== 'available') return;
    set({ phase: 'confirming', error: null });
  },

  async cancelInstallConfirmation() {
    if (get().phase === 'downloading' || get().phase === 'installing') return;
    set({ phase: get().available ? 'available' : 'idle' });
  },

  async downloadAndInstall() {
    const { phase, available, busyWorkKinds } = get();
    if (!available || phase === 'downloading' || phase === 'installing') return;
    if (busyWorkKinds.length > 0) {
      set({
        phase: 'error',
        error: {
          action: 'install',
          message: '检测到有导入、导出或插件任务仍在运行。请先保存并等待任务完成后再安装更新。',
        },
      });
      return;
    }

    const update = updateResource;
    if (!update) {
      set({
        phase: 'error',
        error: { action: 'install', message: '更新资源已释放，请重新检查更新。' },
      });
      return;
    }

    set({ phase: 'downloading', progress: { downloadedBytes: 0, totalBytes: null }, error: null });
    try {
      let downloaded = 0;
      await update.download((event: DownloadEvent) => {
        if (event.event === 'Started') {
          set({
            progress: {
              downloadedBytes: 0,
              totalBytes: event.data.contentLength ?? null,
            },
          });
          return;
        }
        if (event.event === 'Progress') {
          downloaded += event.data.chunkLength;
          set((state) => ({
            progress: {
              downloadedBytes: downloaded,
              totalBytes: state.progress.totalBytes,
            },
          }));
          return;
        }
        set((state) => ({
          progress: {
            downloadedBytes: downloaded,
            totalBytes: state.progress.totalBytes,
          },
        }));
      });

      set({ phase: 'installing' });
      await update.install();
      updateResource = null;
      set({ phase: 'installedNeedsRelaunch' });
    } catch (error) {
      set({
        phase: 'error',
        error: normalizeError(error, get().phase === 'installing' ? 'install' : 'download'),
      });
    }
  },

  async restartNow() {
    await relaunch();
  },

  async resetToAvailable() {
    await closeUpdateResource();
    set({
      phase: 'idle',
      available: null,
      progress: { downloadedBytes: 0, totalBytes: null },
      error: null,
    });
  },

  clearError() {
    set({ error: null });
  },

  beginWork(kind) {
    const token: WorkToken = { kind, id: ++nextTaskId };
    set((state) => {
      const workTokens = [...state.workTokens, token];
      const kinds = new Set(state.busyWorkKinds);
      kinds.add(kind);
      return { workTokens, busyWorkKinds: Array.from(kinds) };
    });
    return token;
  },

  endWork(token) {
    set((state) => {
      const workTokens = state.workTokens.filter((t) => t.id !== token.id);
      const kinds = new Set(workTokens.map((t) => t.kind));
      return { workTokens, busyWorkKinds: Array.from(kinds) };
    });
  },
}));
