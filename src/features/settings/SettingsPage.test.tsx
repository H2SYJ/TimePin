import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import * as ipc from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { DependencyInstallProgress, DependencyReport, InstalledDependencies } from '@/types';
import { SettingsPage } from './SettingsPage';

const { api } = ipc;
const defaults = useAppStore.getState().settings;
const report: DependencyReport = {
  schemaVersion: '1.0.0',
  dependencies: ['ffmpeg', 'ffprobe'].map((id) => ({
    id: id as 'ffmpeg' | 'ffprobe', available: true, path: `C:/tools/${id}.exe`, version: '6.1.1', error: null, minimumVersion: '6.0',
  })),
};
const installed: InstalledDependencies = {
  schemaVersion: '1.0.0', ffmpegPath: 'C:/app/tools/installed/ffmpeg.exe', ffprobePath: 'C:/app/tools/installed/ffprobe.exe', report,
};
let handlers: ((event: DependencyInstallProgress) => void)[];
let unlisten: ReturnType<typeof vi.fn>;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function ready() {
  await waitFor(() => expect(screen.getByRole('button', { name: '检测依赖' })).toBeEnabled());
}

beforeEach(() => {
  handlers = [];
  unlisten = vi.fn();
  useAppStore.setState({ settings: { ...defaults }, error: null, notice: null });
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    matches: false, media: query, onchange: null,
    addListener: () => {}, removeListener: () => {},
    addEventListener: () => {}, removeEventListener: () => {}, dispatchEvent: () => true,
  }));
  vi.spyOn(api, 'saveSettings').mockResolvedValue(undefined);
  vi.spyOn(api, 'getAdvancedSettingsInfo').mockResolvedValue({ schemaVersion: '1.0.0', defaultPluginDirectory: 'C:/app/plugins', installDirectory: 'C:/app/tools/ffmpeg' });
  vi.spyOn(api, 'validateDependencies').mockResolvedValue(report);
  vi.spyOn(api, 'pickDirectory').mockResolvedValue(null);
  vi.spyOn(api, 'pickExecutable').mockResolvedValue(null);
  vi.spyOn(api, 'openPluginDirectory').mockResolvedValue(undefined);
  vi.spyOn(api, 'reloadPlugins').mockResolvedValue([]);
  vi.spyOn(api, 'installDependencies').mockResolvedValue(installed);
  vi.spyOn(api, 'onDependencyInstallProgress').mockImplementation(async (handler) => { handlers.push(handler); return unlisten; });
  vi.spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('advanced settings workspace', () => {
  it('removes M1 placeholder and displays dependency paths only in advanced settings', async () => {
    render(<SettingsPage />);
    await ready();
    expect(screen.queryByText(/M1 阶段/)).not.toBeInTheDocument();
    const advanced = screen.getByLabelText('高级设置');
    expect(within(advanced).getByRole('textbox', { name: /^FFmpeg 路径/ })).toBeInTheDocument();
    expect(within(advanced).getByRole('textbox', { name: /^FFprobe 路径/ })).toBeInTheDocument();
    expect(screen.getAllByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveLength(1);
    expect(screen.getByText(/TimePin 只扫描可执行文件同级 plugins 目录/)).toBeInTheDocument();
    expect(within(screen.getByRole('list', { name: '依赖状态' })).getAllByText(/要求 ≥ 6.0/)).toHaveLength(2);
  });

  it('blocks invalid numeric saves, then allows correction', async () => {
    render(<SettingsPage />);
    await ready();
    fireEvent.change(screen.getByRole('spinbutton', { name: '缩略图质量' }), { target: { value: '999' } });
    fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
    expect(api.saveSettings).not.toHaveBeenCalled();
    expect(screen.getByText(/请先修正以下 1 处/)).toBeInTheDocument();
    expect(screen.getAllByText(/缩略图质量必须是/).length).toBeGreaterThan(0);
    fireEvent.change(screen.getByRole('spinbutton', { name: '缩略图质量' }), { target: { value: '90' } });
    fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
    await waitFor(() => expect(api.saveSettings).toHaveBeenCalledWith(expect.objectContaining({ thumbnailQuality: 90 })));
    expect(vi.mocked(api.saveSettings).mock.calls[0]![0]).not.toHaveProperty('customPluginDirs');
    expect(api.reloadPlugins).not.toHaveBeenCalled();
  });

  it('shows save failure without changing saved state; save and install are disabled during persistence', async () => {
    const saving = deferred<void>();
    vi.mocked(api.saveSettings).mockReturnValueOnce(saving.promise);
    render(<SettingsPage />);
    await ready();
    fireEvent.change(screen.getByRole('textbox', { name: /^FFmpeg 路径/ }), { target: { value: 'D:/missing.exe' } });
    fireEvent.click(screen.getByRole('button', { name: '保存设置' }));
    expect(screen.getByRole('button', { name: /保存中/ })).toBeDisabled();
    expect(screen.getByRole('button', { name: '一键安装依赖' })).toBeDisabled();
    await act(async () => saving.reject(new Error('自定义 FFmpeg 路径不存在')));
    expect(await screen.findByText(/自定义 FFmpeg 路径不存在/)).toBeInTheDocument();
    expect(useAppStore.getState().settings.ffmpegPath).toBeNull();
    expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveValue('D:/missing.exe');
    expect(screen.queryByRole('button', { name: '已保存' })).not.toBeInTheDocument();
  });

  it('displays dependency validation errors inline and invalidates the report after editing paths', async () => {
    render(<SettingsPage />);
    await ready();
    vi.mocked(api.validateDependencies).mockResolvedValueOnce({ ...report, dependencies: report.dependencies.map((dependency) => dependency.id === 'ffmpeg' ? { ...dependency, available: false, version: '5.1', error: 'FFmpeg 版本低于最低支持版本 6.0' } : dependency) });
    fireEvent.change(screen.getByRole('textbox', { name: /^FFmpeg 路径/ }), { target: { value: 'D:/old/ffmpeg.exe' } });
    fireEvent.click(screen.getByRole('button', { name: '检测依赖' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveAttribute('aria-invalid', 'true'));
    expect(screen.getAllByText('FFmpeg 版本低于最低支持版本 6.0').length).toBeGreaterThan(0);
    expect(screen.getByText(/当前 5.1/)).toBeInTheDocument();
    fireEvent.change(screen.getByRole('textbox', { name: /^FFmpeg 路径/ }), { target: { value: 'D:/new/ffmpeg.exe' } });
    expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).not.toHaveAttribute('aria-invalid', 'true');
    expect(screen.queryByText(/当前 5.1/)).not.toBeInTheDocument();
  });

  it('keeps draft untouched when picker is cancelled', async () => {
    render(<SettingsPage />);
    await ready();
    await waitFor(() => expect(screen.getByRole('button', { name: '浏览 FFmpeg' })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: '浏览 FFmpeg' }));
    await waitFor(() => expect(api.pickExecutable).toHaveBeenCalledWith('ffmpeg'));
    expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveValue('');
    expect(screen.getByRole('button', { name: '保存设置' })).toBeDisabled();
  });

  it('shows the unified executable plugins directory and opens it without index', async () => {
    render(<SettingsPage />);
    await ready();
    const advanced = screen.getByLabelText('高级设置');
    expect(within(advanced).getByRole('textbox', { name: /^统一插件目录/ })).toHaveValue('C:/app/plugins');
    fireEvent.click(within(advanced).getByRole('button', { name: '打开目录' }));
    await waitFor(() => expect(api.openPluginDirectory).toHaveBeenCalledWith());
    expect(screen.queryByRole('button', { name: '手动添加目录' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '恢复默认插件目录' })).not.toBeInTheDocument();
  });

  it('correlates progress, disables overlapping operations, and preserves unsaved draft on install success', async () => {
    const pending = deferred<InstalledDependencies>();
    vi.mocked(api.installDependencies).mockReturnValueOnce(pending.promise);
    render(<SettingsPage />);
    await ready();
    fireEvent.change(screen.getByRole('textbox', { name: '默认导入目录' }), { target: { value: 'D:/unsaved' } });
    fireEvent.click(screen.getByRole('button', { name: '一键安装依赖' }));
    await waitFor(() => expect(api.installDependencies).toHaveBeenCalledOnce());
    const taskId = vi.mocked(api.installDependencies).mock.calls[0]![0];
    expect(screen.getByRole('button', { name: '保存设置' })).toBeDisabled();
    const event: DependencyInstallProgress = { schemaVersion: '1.0.0', taskId, stage: 'download', progress: 0.5, message: '下载 ffmpeg', error: null };
    act(() => handlers[0]?.({ ...event, taskId: 'unrelated' }));
    expect(screen.queryByText(/下载 · 50%/)).not.toBeInTheDocument();
    act(() => handlers[0]?.(event));
    expect(screen.getByText(/下载 · 50%/)).toBeInTheDocument();
    act(() => handlers[0]?.({ ...event, stage: 'extract', progress: 0.8, message: '正在解压' }));
    expect(screen.getByText(/解压 · 80%/)).toBeInTheDocument();
    act(() => handlers[0]?.({ ...event, stage: 'validate', progress: 0.95, message: '正在验证' }));
    expect(screen.getByText(/验证 · 95%/)).toBeInTheDocument();
    await act(async () => pending.resolve(installed));
    expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveValue(installed.ffmpegPath);
    expect(screen.getByRole('textbox', { name: /^FFprobe 路径/ })).toHaveValue(installed.ffprobePath);
    expect(screen.getByRole('textbox', { name: '默认导入目录' })).toHaveValue('D:/unsaved');
    expect(useAppStore.getState().settings.importDirectory).toBeNull();
    expect(useAppStore.getState().settings.ffmpegPath).toBe(installed.ffmpegPath);
    expect(api.saveSettings).not.toHaveBeenCalled();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(screen.getByText(/依赖已安装、验证并保存路径/)).toBeInTheDocument();
  });

  it('shows copyable install failure, preserves old paths, and retries using a fresh task id', async () => {
    useAppStore.setState({ settings: { ...defaults, ffmpegPath: 'D:/old/ffmpeg.exe', ffprobePath: 'D:/old/ffprobe.exe' } });
    vi.mocked(api.installDependencies).mockRejectedValueOnce(new Error('SHA256 mismatch'));
    render(<SettingsPage />);
    await ready();
    fireEvent.click(screen.getByRole('button', { name: '一键安装依赖' }));
    const retry = await screen.findByRole('button', { name: '重试安装依赖' });
    expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveValue('D:/old/ffmpeg.exe');
    expect(screen.getByText('SHA256 mismatch')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '复制错误详情' }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('SHA256 mismatch'));
    fireEvent.click(retry);
    await waitFor(() => expect(api.installDependencies).toHaveBeenCalledTimes(2));
    const calls = vi.mocked(api.installDependencies).mock.calls;
    expect(calls[0]![0]).not.toBe(calls[1]![0]);
    await waitFor(() => expect(screen.getByRole('textbox', { name: /^FFmpeg 路径/ })).toHaveValue(installed.ffmpegPath));
    expect(unlisten).toHaveBeenCalledTimes(2);
  });

  it('marks browser preview as desktop-only and never invokes validation or installation', () => {
    vi.spyOn(ipc, 'isTauri').mockReturnValue(false);
    render(<SettingsPage />);
    expect(screen.getByText(/当前为浏览器预览/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '一键安装依赖' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '检测依赖' })).toBeDisabled();
    expect(api.validateDependencies).not.toHaveBeenCalled();
    expect(api.installDependencies).not.toHaveBeenCalled();
    fireEvent.change(screen.getByRole('textbox', { name: '默认导入目录' }), { target: { value: 'D:/browser-draft' } });
    expect(screen.getByRole('button', { name: '保存设置' })).toBeDisabled();
    expect(api.saveSettings).not.toHaveBeenCalled();
  });
});
