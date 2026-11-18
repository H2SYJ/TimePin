import { getCurrentWebview } from '@tauri-apps/api/webview';
import { PhysicalPosition } from '@tauri-apps/api/dpi';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { PluginImportResult, PluginInfo } from '@/types';
import { __resetGitHubDiscoverySessionState } from './GitHubPluginDiscovery';
import { PluginsPage } from './PluginsPage';

type DropHandler = Parameters<ReturnType<typeof getCurrentWebview>['onDragDropEvent']>[0];
const onDragDropEvent = vi.fn<ReturnType<typeof getCurrentWebview>['onDragDropEvent']>();
let dropHandlers: DropHandler[];
let unlisten: ReturnType<typeof vi.fn>;
const plugin: PluginInfo = {
  name: 'test-rule', version: '1.0.0', description: '本地规则', author: 'TimePin',
  runtime: 'python', capabilities: [], parametersSchema: {}, dir: 'C:/plugins/test-rule',
  status: 'ready', statusDetail: null,
};
const imported: PluginImportResult = { schemaVersion: '1.0', name: plugin.name, version: '2.0.0', replaced: false };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}

async function mountPage() {
  const view = render(<PluginsPage />);
  await screen.findByRole('heading', { name: plugin.name });
  return view;
}

function nativeDrop(paths: string[], handler?: DropHandler) {
  const target = handler ?? dropHandlers[dropHandlers.length - 1];
  if (!target) throw new Error('尚无已注册的原生拖放监听');
  act(() => target({
    event: 'tauri://drag-drop', id: 1,
    payload: { type: 'drop', paths, position: new PhysicalPosition(0, 0) },
  }));
}

function htmlDrop(files: File[]) {
  fireEvent.drop(screen.getByRole('region', { name: '插件包拖放区域' }), { dataTransfer: { files } });
}

beforeEach(() => {
  localStorage.clear();
  __resetGitHubDiscoverySessionState();
  useAppStore.setState({
    ...useAppStore.getInitialState(), page: 'plugins',
    settings: { ...useAppStore.getInitialState().settings, customPluginDirs: ['C:/plugins', 'D:/other'] },
  });
  vi.spyOn(api, 'getPlugins').mockResolvedValue([plugin]);
  vi.spyOn(api, 'reloadPlugins').mockResolvedValue([plugin]);
  vi.spyOn(api, 'pickPluginPackage').mockResolvedValue('C:/packages/rule.tppkg');
  vi.spyOn(api, 'importPlugin').mockResolvedValue(imported);
  vi.spyOn(api, 'onPluginProgress').mockResolvedValue(() => {});
  vi.spyOn(api, 'onPluginStatus').mockResolvedValue(() => {});
  vi.spyOn(api, 'onPluginLog').mockResolvedValue(() => {});
  vi.spyOn(api, 'onPluginResult').mockResolvedValue(() => {});
  vi.spyOn(api, 'getGitHubSession').mockResolvedValue({
    schemaVersion: '1.0.0', identity: 'ANONYMOUS', remembered: false, storageWarning: null, rateLimit: null,
  });
  vi.spyOn(api, 'discoverGitHubPlugins').mockResolvedValue({ schemaVersion: '1.0.0', plugins: [], totalCount: 0, rateLimit: null });
  dropHandlers = [];
  unlisten = vi.fn();
  onDragDropEvent.mockReset().mockImplementation(async (handler) => {
    dropHandlers.push(handler);
    return unlisten;
  });
  vi.mocked(getCurrentWebview).mockReturnValue({ onDragDropEvent } as unknown as ReturnType<typeof getCurrentWebview>);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  useAppStore.setState(useAppStore.getInitialState());
  __resetGitHubDiscoverySessionState();
});

describe('本地插件包导入', () => {
  it('提供可聚焦的按钮、明确拖放区域、目标目录及信任/覆盖/停用警告', async () => {
    await mountPage();
    const button = screen.getByRole('button', { name: '导入插件' });
    act(() => button.focus());
    expect(button).toHaveFocus();
    expect(screen.getByRole('region', { name: '插件包拖放区域' })).toBeVisible();
    expect(screen.getByText(/支持 .tppkg \/ .zip，一次一个/)).toBeVisible();
    expect(screen.getByText(/仅导入信任的包.*覆盖同名插件.*停止所有正在运行的插件/)).toBeVisible();
    expect(screen.getByText('导入到第一个已保存的自定义插件目录：C:/plugins')).toBeVisible();
    expect(screen.queryByText(/D:\/other/)).not.toBeInTheDocument();
    fireEvent.keyDown(button, { key: 'Enter', code: 'Enter' });
    fireEvent.keyUp(button, { key: 'Enter', code: 'Enter' });
    await waitFor(() => expect(api.pickPluginPackage).toHaveBeenCalledTimes(1));
    await screen.findByText(/已导入插件 test-rule v2.0.0/);
  });

  it('选择框取消不发起导入、不刷新且不报错', async () => {
    vi.mocked(api.pickPluginPackage).mockResolvedValueOnce(null);
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    await waitFor(() => expect(screen.getByRole('button', { name: '导入插件' })).toBeEnabled());
    expect(api.pickPluginPackage).toHaveBeenCalledOnce();
    expect(api.importPlugin).not.toHaveBeenCalled();
    expect(api.getPlugins).toHaveBeenCalledOnce();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('导入路径、刷新并显示更新后的版本和同名替换反馈，不重复 reload', async () => {
    vi.mocked(api.importPlugin).mockResolvedValueOnce({ ...imported, replaced: true });
    vi.mocked(api.getPlugins).mockResolvedValueOnce([plugin]).mockResolvedValue([{ ...plugin, version: '2.0.0', description: '新版规则' }]);
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(await screen.findByText('已替换同名插件 test-rule v2.0.0。')).toHaveAttribute('role', 'status');
    expect(api.importPlugin).toHaveBeenCalledWith('C:/packages/rule.tppkg');
    expect(screen.getByText('新版规则 · v2.0.0 · TimePin')).toBeVisible();
    expect(api.getPlugins).toHaveBeenCalledTimes(2);
    expect(api.reloadPlugins).not.toHaveBeenCalled();
  });

  it.each(['插件包清单无效', new Error('无法读取文件')])('显示后端或选择框失败并可重试：%s', async (cause) => {
    vi.mocked(api.importPlugin).mockRejectedValueOnce(cause);
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(/导入失败.*(清单无效|无法读取文件)/);
    expect(screen.getByRole('button', { name: '导入插件' })).toBeEnabled();
    expect(api.getPlugins).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(await screen.findByText('已导入插件 test-rule v2.0.0。')).toBeVisible();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('选择框失败也有可见反馈并解锁', async () => {
    vi.mocked(api.pickPluginPackage).mockRejectedValueOnce(new Error('选择框不可用'));
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('选择框不可用');
    expect(api.importPlugin).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: '导入插件' })).toBeEnabled();
  });

  it('互斥锁覆盖选择框、HTML/原生拖放、IPC 及刷新全过程', async () => {
    const picking = deferred<string | null>();
    const installing = deferred<PluginImportResult>();
    const refreshing = deferred<PluginInfo[]>();
    vi.mocked(api.pickPluginPackage).mockReturnValueOnce(picking.promise);
    vi.mocked(api.importPlugin).mockReturnValueOnce(installing.promise);
    vi.mocked(api.getPlugins).mockResolvedValueOnce([plugin]).mockReturnValueOnce(refreshing.promise);
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    const importButton = () => screen.getByText('正在导入…').closest('button');
    await waitFor(() => expect(importButton()).toBeDisabled());
    expect(screen.getByRole('button', { name: '重新加载' })).toBeDisabled();
    nativeDrop(['other.zip']);
    htmlDrop([new File(['zip'], 'other.zip')]);
    expect(api.importPlugin).not.toHaveBeenCalled();
    await act(async () => picking.resolve('rule.zip'));
    nativeDrop(['other.zip']);
    htmlDrop([new File(['zip'], 'other.zip')]);
    expect(api.importPlugin).toHaveBeenCalledTimes(1);
    await act(async () => installing.resolve(imported));
    expect(importButton()).toBeDisabled();
    nativeDrop(['other.zip']);
    expect(api.importPlugin).toHaveBeenCalledTimes(1);
    await act(async () => refreshing.resolve([plugin]));
    expect(screen.getByRole('button', { name: '导入插件' })).toBeEnabled();
    expect(api.pickPluginPackage).toHaveBeenCalledOnce();
  });

  it.each(['ready', 'crashed'] as const)('导入时禁用插件运行/重试，避免与替换竞争：%s', async (status) => {
    const picking = deferred<string | null>();
    vi.mocked(api.pickPluginPackage).mockReturnValueOnce(picking.promise);
    vi.mocked(api.getPlugins).mockResolvedValue([{ ...plugin, status }]);
    const run = vi.spyOn(api, 'runPlugin');
    useAppStore.setState({
      videos: [{
        id: 'video', name: 'clip.mp4', path: 'C:/clip.mp4', sourcePath: 'C:/clip.mp4',
        duration: 1, width: 1920, height: 1080, fps: 30, codec: 'h264', fileSize: 1,
        importedAt: '2026-11-18', thumbnailPath: null, sidecarTxtPath: null,
        sidecarCsvPath: null, sidecarXmlPath: null, tags: [], metadata: {},
      }],
    });
    await mountPage();
    const action = () => screen.getByRole('button', { name: status === 'ready' ? '运行' : '重试' });
    expect(action()).toBeEnabled();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(action()).toBeDisabled();
    fireEvent.click(action());
    expect(run).not.toHaveBeenCalled();
    await act(async () => picking.resolve(null));
    expect(action()).toBeEnabled();
  });

  it.each([{ dirs: [] as string[] }, { dirs: ['', 'D:/other'] }])('没有有效的第一个已保存目录时引导设置且不选择/读取/导入', async ({ dirs }) => {
    useAppStore.setState((state) => ({ settings: { ...state.settings, customPluginDirs: dirs } }));
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(screen.getByRole('alert')).toHaveTextContent('添加并保存第一个自定义插件目录');
    nativeDrop(['rule.zip']);
    htmlDrop([new File(['zip'], 'rule.zip')]);
    expect(api.pickPluginPackage).not.toHaveBeenCalled();
    expect(api.importPlugin).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '前往设置' }));
    expect(useAppStore.getState().page).toBe('settings');
  });

  it('设置保存失败的候选目录不作为有效导入配置', async () => {
    useAppStore.setState((state) => ({ settings: { ...state.settings, customPluginDirs: [] } }));
    vi.spyOn(api, 'saveSettings').mockRejectedValueOnce('无法保存');
    await useAppStore.getState().updateSettings({ ...useAppStore.getState().settings, customPluginDirs: ['C:/draft'] });
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(screen.getByRole('alert')).toHaveTextContent('前往设置');
    expect(api.pickPluginPackage).not.toHaveBeenCalled();
    expect(api.importPlugin).not.toHaveBeenCalled();
  });

  it('原生拖放使用路径（大小写不敏感），不触发 FileReader', async () => {
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    await mountPage();
    nativeDrop(['C:\\packages\\rule.TPPKG']);
    expect(await screen.findByText('已导入插件 test-rule v2.0.0。')).toBeVisible();
    expect(api.importPlugin).toHaveBeenCalledWith('C:\\packages\\rule.TPPKG');
    expect(read).not.toHaveBeenCalled();
    expect(api.pickPluginPackage).not.toHaveBeenCalled();
  });

  it('HTML File 以 FileReader base64 导入，去掉 data URL 前缀', async () => {
    await mountPage();
    htmlDrop([new File(['zip'], 'rule.ZIP', { type: 'application/zip' })]);
    await screen.findByText('已导入插件 test-rule v2.0.0。');
    expect(api.importPlugin).toHaveBeenCalledWith({ base64: 'emlw', fileName: 'rule.ZIP' });
  });

  it('HTML 超过 128 MiB 时在 FileReader 前拒绝', async () => {
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    const file = new File([''], 'large.zip');
    Object.defineProperty(file, 'size', { value: 128 * 1024 * 1024 + 1 });
    await mountPage();
    htmlDrop([file]);
    expect(await screen.findByRole('alert')).toHaveTextContent('不能超过 128 MiB');
    expect(read).not.toHaveBeenCalled();
    expect(api.importPlugin).not.toHaveBeenCalled();
  });

  it('HTML 读取失败有可见错误且可以重试', async () => {
    vi.spyOn(FileReader.prototype, 'readAsDataURL').mockImplementationOnce(function (this: FileReader) {
      this.dispatchEvent(new ProgressEvent('error'));
    });
    await mountPage();
    htmlDrop([new File(['zip'], 'rule.zip')]);
    expect(await screen.findByRole('alert')).toHaveTextContent('读取插件包失败');
    expect(api.importPlugin).not.toHaveBeenCalled();
    expect(screen.getByRole('button', { name: '导入插件' })).toBeEnabled();
  });

  it.each(['picker', 'native', 'html'] as const)('拒绝无效扩展名：%s', async (entry) => {
    const read = vi.spyOn(FileReader.prototype, 'readAsDataURL');
    await mountPage();
    if (entry === 'picker') {
      vi.mocked(api.pickPluginPackage).mockResolvedValueOnce('rule.exe');
      fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    } else if (entry === 'native') nativeDrop(['rule.exe']);
    else htmlDrop([new File([''], 'rule.exe')]);
    expect(await screen.findByRole('alert')).toHaveTextContent('仅支持 .tppkg 或 .zip');
    expect(api.importPlugin).not.toHaveBeenCalled();
    expect(read).not.toHaveBeenCalled();
  });

  it.each(['native', 'html'] as const)('拒绝多文件且不静默取第一个：%s', async (entry) => {
    await mountPage();
    if (entry === 'native') nativeDrop(['a.zip', 'b.zip']);
    else htmlDrop([new File([''], 'a.zip'), new File([''], 'b.zip')]);
    expect(await screen.findByRole('alert')).toHaveTextContent('一次只能导入一个插件包');
    expect(api.importPlugin).not.toHaveBeenCalled();
  });

  it('安装完成但列表刷新失败时保留成功事实并提示重新加载', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    vi.mocked(api.getPlugins).mockResolvedValueOnce([plugin]).mockRejectedValueOnce('列表读取失败');
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('已导入插件 test-rule v2.0.0。但列表刷新失败，请点击「重新加载」重试。');
    expect(screen.getByRole('alert')).not.toHaveTextContent('导入失败');
    expect(screen.getByRole('heading', { name: plugin.name })).toBeVisible();
    expect(screen.getByRole('button', { name: '重新加载' })).toBeEnabled();
  });

  it('切至发现 Tab 清理监听且忽略陈旧原生事件，返回本地重新注册', async () => {
    const view = await mountPage();
    const staleHandler = dropHandlers[0];
    fireEvent.click(screen.getByRole('tab', { name: '发现插件' }));
    expect(unlisten).toHaveBeenCalledOnce();
    expect(screen.queryByRole('button', { name: '导入插件' })).not.toBeInTheDocument();
    nativeDrop(['rule.zip'], staleHandler);
    expect(api.importPlugin).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('tab', { name: '本地插件' }));
    await screen.findByRole('heading', { name: plugin.name });
    expect(onDragDropEvent).toHaveBeenCalledTimes(2);
    nativeDrop(['rule.zip']);
    await screen.findByText('已导入插件 test-rule v2.0.0。');
    view.unmount();
    expect(unlisten).toHaveBeenCalledTimes(2);
  });

  it.each(['tab', 'unmount'] as const)('监听注册尚未完成时离开也正确异步清理：%s', async (exit) => {
    const registering = deferred<() => void>();
    const delayedCleanup = vi.fn();
    onDragDropEvent.mockImplementationOnce((handler) => {
      dropHandlers.push(handler);
      return registering.promise;
    });
    const view = await mountPage();
    if (exit === 'tab') fireEvent.click(screen.getByRole('tab', { name: '发现插件' }));
    else view.unmount();
    await act(async () => registering.resolve(delayedCleanup));
    expect(delayedCleanup).toHaveBeenCalledOnce();
    nativeDrop(['rule.zip'], dropHandlers[0]);
    expect(api.importPlugin).not.toHaveBeenCalled();
  });

  it('选择框在切换 Tab 后返回时不导入、不强制切回', async () => {
    const picking = deferred<string | null>();
    vi.mocked(api.pickPluginPackage).mockReturnValueOnce(picking.promise);
    await mountPage();
    fireEvent.click(screen.getByRole('button', { name: '导入插件' }));
    fireEvent.click(screen.getByRole('tab', { name: '发现插件' }));
    await act(async () => picking.resolve('rule.zip'));
    expect(api.importPlugin).not.toHaveBeenCalled();
    expect(screen.getByRole('tab', { name: '发现插件' })).toHaveAttribute('aria-selected', 'true');
  });
});
