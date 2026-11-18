import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';

vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }));
beforeEach(() => { vi.mocked(invoke).mockReset(); vi.mocked(open).mockReset(); vi.mocked(listen).mockReset(); });
afterEach(() => { vi.unstubAllGlobals(); });

describe('advanced settings IPC contract', () => {
  it('passes camelCase payloads and opens the unified directory without arguments', async () => {
    const settings = { ...useAppStore.getState().settings, ffprobePath: 'D:/ffprobe.exe' };
    await api.getAdvancedSettingsInfo();
    expect(invoke).toHaveBeenLastCalledWith('get_advanced_settings_info');
    await api.validateDependencies(settings);
    expect(invoke).toHaveBeenLastCalledWith('validate_dependencies', { settings });
    await api.saveSettings(settings);
    expect(invoke).toHaveBeenLastCalledWith('save_settings', { settings });
    await api.openPluginDirectory();
    expect(invoke).toHaveBeenLastCalledWith('open_plugin_directory');
    await api.installDependencies('attempt-1');
    expect(invoke).toHaveBeenLastCalledWith('install_dependencies', { taskId: 'attempt-1' });
  });

  it('preserves picker cancellation and returns selected files and directories', async () => {
    vi.mocked(open).mockResolvedValueOnce(null).mockResolvedValueOnce('D:/plugins').mockResolvedValueOnce('D:/export').mockResolvedValueOnce('D:/ffmpeg.exe');
    expect(await api.pickExecutable('ffprobe')).toBeNull();
    expect(await api.pickDirectory()).toBe('D:/plugins');
    expect(open).toHaveBeenLastCalledWith(expect.objectContaining({ directory: true, multiple: false, title: '选择目录' }));
    expect(await api.pickDirectory('选择导出目录')).toBe('D:/export');
    expect(open).toHaveBeenLastCalledWith(expect.objectContaining({ directory: true, multiple: false, title: '选择导出目录' }));
    expect(await api.pickExecutable('ffmpeg')).toBe('D:/ffmpeg.exe');
    expect(open).toHaveBeenLastCalledWith(expect.objectContaining({ multiple: false }));
  });

  it('subscribes to the versioned dependency progress channel', async () => {
    const unlisten = vi.fn();
    vi.mocked(listen).mockResolvedValueOnce(unlisten);
    const callback = vi.fn();
    expect(await api.onDependencyInstallProgress(callback)).toBe(unlisten);
    expect(listen).toHaveBeenCalledWith('dependency://install-progress', expect.any(Function));
  });

  it('throws desktop-only errors in browser instead of pretending save, validation or installation succeeded', async () => {
    vi.stubGlobal('window', {});
    const settings = useAppStore.getState().settings;
    await expect(api.saveSettings(settings)).rejects.toThrow(/桌面应用/);
    await expect(api.getAdvancedSettingsInfo()).rejects.toThrow(/桌面应用/);
    await expect(api.validateDependencies(settings)).rejects.toThrow(/桌面应用/);
    await expect(api.installDependencies('attempt-1')).rejects.toThrow(/桌面应用/);
    await expect(api.onDependencyInstallProgress(vi.fn())).rejects.toThrow(/桌面应用/);
    await expect(api.openPluginDirectory()).rejects.toThrow(/桌面应用/);
    await expect(api.pickDirectory()).rejects.toThrow(/桌面应用/);
    await expect(api.pickExecutable('ffmpeg')).rejects.toThrow(/桌面应用/);
    expect(invoke).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });
});
