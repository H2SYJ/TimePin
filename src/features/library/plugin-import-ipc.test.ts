import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/lib/ipc';
import type { PluginImportResult } from '@/types';

beforeEach(() => {
  vi.mocked(invoke).mockReset();
  vi.mocked(open).mockReset();
});
afterEach(() => vi.unstubAllGlobals());

describe('local plugin package IPC contract', () => {
  it.each([
    'C:/packages/rule.tppkg',
    { base64: 'UEsDBA==', fileName: 'rule.zip' },
  ])('sends the source unchanged with schemaVersion 1.0', async (source) => {
    const result: PluginImportResult = { schemaVersion: '1.0', name: 'rule', version: '2.0.0', replaced: true };
    vi.mocked(invoke).mockResolvedValueOnce(result);
    await expect(api.importPlugin(source)).resolves.toEqual(result);
    expect(invoke).toHaveBeenLastCalledWith('import_plugin', { source, schemaVersion: '1.0' });
  });

  it('preserves backend string failures', async () => {
    vi.mocked(invoke).mockRejectedValueOnce('插件包清单无效');
    await expect(api.importPlugin('bad.zip')).rejects.toBe('插件包清单无效');
  });

  it('opens a single-file native picker filtered to supported packages', async () => {
    vi.mocked(open).mockResolvedValueOnce('C:/packages/rule.TPPKG');
    await expect(api.pickPluginPackage()).resolves.toBe('C:/packages/rule.TPPKG');
    expect(open).toHaveBeenLastCalledWith({
      multiple: false, title: '选择插件包',
      filters: [{ name: 'TimePin 插件包', extensions: ['tppkg', 'zip'] }],
    });
    expect(invoke).not.toHaveBeenCalled();
  });

  it('returns null for picker cancellation and normalizes a single path array', async () => {
    vi.mocked(open).mockResolvedValueOnce(null).mockResolvedValueOnce([]).mockResolvedValueOnce(['rule.zip']);
    await expect(api.pickPluginPackage()).resolves.toBeNull();
    await expect(api.pickPluginPackage()).resolves.toBeNull();
    await expect(api.pickPluginPackage()).resolves.toBe('rule.zip');
  });

  it('rejects multiple picker results rather than silently choosing the first', async () => {
    vi.mocked(open).mockResolvedValueOnce(['a.zip', 'b.zip']);
    await expect(api.pickPluginPackage()).rejects.toThrow('一次只能导入一个插件包');
  });

  it('does not allow browser preview to select or install local packages', async () => {
    vi.stubGlobal('window', {});
    await expect(api.importPlugin('rule.zip')).rejects.toThrow(/桌面应用/);
    await expect(api.pickPluginPackage()).rejects.toThrow(/桌面应用/);
    expect(invoke).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });
});
