import { beforeEach, describe, expect, it, vi } from 'vitest';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getVersion } from '@tauri-apps/api/app';
import { api, isTauri } from '@/lib/ipc';
import { useUpdateStore } from './updater';

vi.mock('@tauri-apps/api/app', () => ({
  getVersion: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-updater', () => ({
  check: vi.fn(),
}));

vi.mock('@tauri-apps/plugin-process', () => ({
  relaunch: vi.fn(),
}));

vi.mock('@/lib/ipc', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/ipc')>();
  return {
    ...actual,
    isTauri: vi.fn(() => true),
    api: {
      ...actual.api,
      getUpdaterConfig: vi.fn(),
    },
  };
});

function makeUpdate(overrides: Partial<Record<'version' | 'currentVersion' | 'body' | 'date', string>> = {}) {
  const callbacks: Array<(event: any) => void> = [];
  return {
    currentVersion: overrides.currentVersion ?? '0.0.6',
    version: overrides.version ?? '0.0.7',
    date: overrides.date,
    body: overrides.body,
    rawJson: {},
    download: vi.fn(async (onEvent?: (event: any) => void) => {
      if (onEvent) callbacks.push(onEvent);
      onEvent?.({ event: 'Started', data: { contentLength: 100 } });
      onEvent?.({ event: 'Progress', data: { chunkLength: 40 } });
      onEvent?.({ event: 'Progress', data: { chunkLength: 60 } });
      onEvent?.({ event: 'Finished' });
    }),
    install: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
  };
}

const mockedCheck = vi.mocked(check);
const mockedGetVersion = vi.mocked(getVersion);
const mockedRelaunch = vi.mocked(relaunch);
const mockedConfig = vi.mocked(api.getUpdaterConfig);
const mockedIsTauri = vi.mocked(isTauri);

describe('useUpdateStore', () => {
  beforeEach(async () => {
    await useUpdateStore.getState().resetToAvailable();
    useUpdateStore.setState({
      phase: 'idle',
      currentVersion: '',
      available: null,
      progress: { downloadedBytes: 0, totalBytes: null },
      error: null,
      busyWorkKinds: [],
      workTokens: [],
    });
    vi.clearAllMocks();
    mockedIsTauri.mockReturnValue(true);
    mockedGetVersion.mockResolvedValue('0.0.6');
    mockedConfig.mockResolvedValue({ schemaVersion: '1.0.0', configured: true });
  });

  it('marks browser preview as desktop only', async () => {
    mockedIsTauri.mockReturnValue(false);
    await useUpdateStore.getState().checkForUpdate();
    expect(useUpdateStore.getState().phase).toBe('desktopOnly');
    expect(mockedCheck).not.toHaveBeenCalled();
  });

  it('reports notConfigured instead of up to date when backend config is missing', async () => {
    mockedConfig.mockResolvedValue({ schemaVersion: '1.0.0', configured: false });
    await useUpdateStore.getState().checkForUpdate();
    const state = useUpdateStore.getState();
    expect(state.phase).toBe('notConfigured');
    expect(state.error?.message).toContain('TAURI_UPDATER_PUBLIC_KEY');
    expect(mockedCheck).not.toHaveBeenCalled();
  });

  it('stores an available update and prevents duplicate checks', async () => {
    const update = makeUpdate();
    mockedCheck.mockResolvedValue(update as any);
    const first = useUpdateStore.getState().checkForUpdate();
    const second = useUpdateStore.getState().checkForUpdate();
    await Promise.all([first, second]);
    const state = useUpdateStore.getState();
    expect(state.phase).toBe('available');
    expect(state.available?.version).toBe('0.0.7');
    expect(state.currentVersion).toBe('0.0.6');
    expect(mockedCheck).toHaveBeenCalledTimes(1);
  });

  it('transitions to upToDate when no update is returned', async () => {
    mockedCheck.mockResolvedValue(null);
    await useUpdateStore.getState().checkForUpdate();
    expect(useUpdateStore.getState().phase).toBe('upToDate');
  });

  it('blocks install while tracked work is running', async () => {
    const update = makeUpdate();
    mockedCheck.mockResolvedValue(update as any);
    await useUpdateStore.getState().checkForUpdate();
    useUpdateStore.getState().beginWork('plugin');
    useUpdateStore.getState().requestInstallConfirmation();
    await useUpdateStore.getState().downloadAndInstall();
    const state = useUpdateStore.getState();
    expect(state.phase).toBe('error');
    expect(state.error?.message).toContain('任务');
    expect(update.download).not.toHaveBeenCalled();
  });

  it('keeps install blocked until all same-kind tasks finish', async () => {
    const update = makeUpdate();
    mockedCheck.mockResolvedValue(update as any);
    await useUpdateStore.getState().checkForUpdate();

    const first = useUpdateStore.getState().beginWork('plugin');
    const second = useUpdateStore.getState().beginWork('plugin');
    expect(useUpdateStore.getState().busyWorkKinds).toContain('plugin');

    // First task ends but second is still running — must stay blocked.
    useUpdateStore.getState().endWork(first);
    expect(useUpdateStore.getState().busyWorkKinds).toContain('plugin');
    useUpdateStore.getState().requestInstallConfirmation();
    await useUpdateStore.getState().downloadAndInstall();
    expect(useUpdateStore.getState().phase).toBe('error');
    expect(update.download).not.toHaveBeenCalled();

    // All tasks done — install is allowed again.
    useUpdateStore.getState().endWork(second);
    expect(useUpdateStore.getState().busyWorkKinds).not.toContain('plugin');
  });

  it('downloads with progress, installs, and releases resource', async () => {
    const update = makeUpdate();
    mockedCheck.mockResolvedValue(update as any);
    await useUpdateStore.getState().checkForUpdate();
    useUpdateStore.getState().requestInstallConfirmation();
    await useUpdateStore.getState().downloadAndInstall();
    const state = useUpdateStore.getState();
    expect(state.phase).toBe('installedNeedsRelaunch');
    expect(state.progress).toEqual({ downloadedBytes: 100, totalBytes: 100 });
    expect(update.download).toHaveBeenCalledTimes(1);
    expect(update.install).toHaveBeenCalledTimes(1);
  });

  it('surfaces download errors and keeps resource for retry', async () => {
    const update = makeUpdate();
    update.download.mockRejectedValue(new Error('network timeout'));
    mockedCheck.mockResolvedValue(update as any);
    await useUpdateStore.getState().checkForUpdate();
    useUpdateStore.getState().requestInstallConfirmation();
    await useUpdateStore.getState().downloadAndInstall();
    const state = useUpdateStore.getState();
    expect(state.phase).toBe('error');
    expect(state.error?.message).toContain('无法连接更新服务器');
    expect(state.available?.version).toBe('0.0.7');
  });

  it('normalizes signature failures', async () => {
    mockedCheck.mockRejectedValue(new Error('invalid signature'));
    await useUpdateStore.getState().checkForUpdate();
    expect(useUpdateStore.getState().phase).toBe('error');
    expect(useUpdateStore.getState().error?.message).toContain('签名校验失败');
  });

  it('resetToAvailable closes held resources', async () => {
    const update = makeUpdate();
    mockedCheck.mockResolvedValue(update as any);
    await useUpdateStore.getState().checkForUpdate();
    await useUpdateStore.getState().resetToAvailable();
    expect(update.close).toHaveBeenCalledTimes(1);
    expect(useUpdateStore.getState().phase).toBe('idle');
  });

  it('restartNow delegates to plugin-process relaunch', async () => {
    await useUpdateStore.getState().restartNow();
    expect(mockedRelaunch).toHaveBeenCalledTimes(1);
  });
});
