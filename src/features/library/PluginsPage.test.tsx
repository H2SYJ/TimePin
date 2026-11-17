import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { api } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { DiscoveredPlugin, GitHubSession, PluginInfo, VideoInfo } from '@/types';
import { __resetGitHubDiscoverySessionState } from './GitHubPluginDiscovery';
import { PluginsPage } from './PluginsPage';

const localPlugin: PluginInfo = {
  name: 'local-plugin', version: '1.0.0', description: '本地测试规则', author: 'TimePin',
  runtime: 'python', capabilities: ['metadata'], dir: 'C:/plugins/local-plugin',
  status: 'ready', statusDetail: null,
  parametersSchema: {
    label: { type: 'string', title: '标注名称', default: '默认名称' },
    threshold: { type: 'number', title: '置信度阈值', default: 0.5 },
  },
};

const discoveredPlugin: DiscoveredPlugin = {
  repository: {
    name: 'community-plugin', fullName: 'owner/community-plugin', description: '社区测试规则',
    htmlUrl: 'https://github.com/owner/community-plugin',
    cloneUrl: 'https://github.com/owner/community-plugin.git',
    stargazersCount: 10, forksCount: 2, updatedAt: '2026-01-01T00:00:00Z',
    language: 'Python', topics: ['time-pin-plugins'], defaultBranch: 'main',
    owner: { login: 'owner', avatarUrl: '', htmlUrl: 'https://github.com/owner' },
  },
  manifest: {
    protocolVersion: '1.0.0', name: 'community-plugin', version: '1.0.0',
    description: '社区测试规则', author: 'owner', entry: 'main.py', runtime: 'python',
    capabilities: ['metadata'], parametersSchema: {}, outputSchema: {},
  },
  manifestError: null, isInstalled: false, localPath: null, verification: 'community',
};

const installedPlugin: PluginInfo = {
  ...localPlugin, name: 'community-plugin', description: '社区测试规则',
  dir: 'C:/plugins/community-plugin', parametersSchema: {},
};

const anonymousSession: GitHubSession = {
  schemaVersion: '1.0.0', identity: 'ANONYMOUS', remembered: false,
  storageWarning: null, rateLimit: null,
};

const video: VideoInfo = {
  id: 'selected-video', name: 'recording.mp4', path: 'C:/videos/recording.mp4',
  sourcePath: 'C:/videos/recording.mp4', duration: 100, width: 1920, height: 1080,
  fps: 30, codec: 'h264', fileSize: 100, importedAt: '2026-01-01',
  thumbnailPath: null, sidecarTxtPath: null, sidecarCsvPath: null, sidecarXmlPath: null,
  tags: [], metadata: {},
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}

beforeEach(() => {
  localStorage.clear();
  __resetGitHubDiscoverySessionState();
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    matches: false, media: query, onchange: null,
    addListener: () => {}, removeListener: () => {},
    addEventListener: () => {}, removeEventListener: () => {}, dispatchEvent: () => true,
  }));
  useAppStore.setState({
    videos: [video], selectedVideoId: video.id, page: 'plugins',
    settings: { ...useAppStore.getInitialState().settings, importDirectory: 'C:/exports' },
  });
  vi.spyOn(api, 'getPlugins').mockResolvedValue([localPlugin]);
  vi.spyOn(api, 'reloadPlugins').mockResolvedValue([localPlugin]);
  vi.spyOn(api, 'onPluginProgress').mockResolvedValue(() => {});
  vi.spyOn(api, 'onPluginStatus').mockResolvedValue(() => {});
  vi.spyOn(api, 'onPluginLog').mockResolvedValue(() => {});
  vi.spyOn(api, 'onPluginResult').mockResolvedValue(() => {});
  vi.spyOn(api, 'getGitHubSession').mockResolvedValue(anonymousSession);
  vi.spyOn(api, 'discoverGitHubPlugins').mockResolvedValue({
    schemaVersion: '1.0.0', plugins: [discoveredPlugin], totalCount: 1, rateLimit: null,
  });
  vi.spyOn(api, 'installGitHubPlugin').mockResolvedValue('C:/plugins/community-plugin');
  vi.spyOn(api, 'runPlugin').mockResolvedValue({
    protocolVersion: '1.0.0', pluginName: localPlugin.name, marks: [], logs: [], evidencePaths: [],
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  useAppStore.setState(useAppStore.getInitialState());
  __resetGitHubDiscoverySessionState();
});

async function openDiscovery() {
  fireEvent.click(screen.getByRole('tab', { name: '发现插件' }));
  await screen.findByRole('heading', { name: 'community-plugin' });
  await act(async () => {});
}

describe('插件管理本地/发现导航', () => {
  it('默认显示本地插件、参数和运行入口，不提前发起 GitHub 请求', async () => {
    render(<PluginsPage />);
    expect(await screen.findByRole('heading', { name: localPlugin.name })).toBeVisible();
    expect(screen.getByRole('tab', { name: '本地插件' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: '发现插件' })).toHaveAttribute('aria-selected', 'false');
    expect(screen.getByDisplayValue('默认名称')).toBeVisible();
    expect(screen.getByRole('button', { name: '运行' })).toBeEnabled();
    expect(api.getGitHubSession).not.toHaveBeenCalled();
    expect(api.discoverGitHubPlugins).not.toHaveBeenCalled();
  });

  it('发现入口挂载真实发现界面并自动搜索 topic，同时保留搜索与 Token 入口', async () => {
    render(<PluginsPage />);
    await screen.findByRole('heading', { name: localPlugin.name });
    await openDiscovery();
    expect(screen.getByRole('tab', { name: '发现插件' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('heading', { name: '发现插件' })).toBeVisible();
    expect(screen.getByText('GitHub (topic: time-pin-plugins)')).toBeVisible();
    expect(api.getGitHubSession).toHaveBeenCalled();
    // IPC 的默认发现端点由后端固定搜索 time-pin-plugins topic。
    expect(api.discoverGitHubPlugins).toHaveBeenCalledWith(1, 20);
    expect(screen.getByRole('button', { name: '刷新' })).toBeEnabled();
    expect(screen.getByRole('button', { name: /配置 Token/ })).toBeEnabled();
    expect(screen.getByRole('button', { name: '安装' })).toBeEnabled();
    const discoveryPane = screen.getByRole('tabpanel', { name: '发现插件' });
    expect(within(discoveryPane).queryByRole('button', { name: '运行' })).not.toBeInTheDocument();
  });

  it('本地初次加载尚未完成时也能进入发现并完成搜索', async () => {
    const loading = deferred<PluginInfo[]>();
    vi.mocked(api.getPlugins).mockReturnValueOnce(loading.promise);
    render(<PluginsPage />);
    expect(screen.getByText('加载插件中…')).toBeVisible();
    await openDiscovery();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledWith(1, 20);
    await act(async () => loading.resolve([localPlugin]));
    expect(screen.getByRole('tab', { name: '发现插件' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('heading', { name: 'community-plugin' })).toBeVisible();
  });

  it('本地重新加载缓慢时不阻塞发现导航，返回结果不会强制切回本地', async () => {
    const reloading = deferred<PluginInfo[]>();
    vi.mocked(api.reloadPlugins).mockReturnValueOnce(reloading.promise);
    render(<PluginsPage />);
    await screen.findByRole('heading', { name: localPlugin.name });
    fireEvent.click(screen.getByRole('button', { name: '重新加载' }));
    expect(api.reloadPlugins).toHaveBeenCalled();
    await openDiscovery();
    await act(async () => reloading.resolve([localPlugin]));
    expect(screen.getByRole('tab', { name: '发现插件' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('heading', { name: 'community-plugin' })).toBeVisible();
  });

  it('在发现界面确认安装后，返回本地刷新列表并显示新插件', async () => {
    let installed = false;
    vi.mocked(api.getPlugins).mockImplementation(async () => installed ? [installedPlugin] : []);
    vi.mocked(api.installGitHubPlugin).mockImplementation(async () => {
      installed = true;
      return installedPlugin.dir;
    });
    render(<PluginsPage />);
    await screen.findByRole('heading', { name: '未发现插件' });
    await openDiscovery();
    fireEvent.click(screen.getByRole('button', { name: '安装' }));
    const dialog = screen.getByRole('dialog', { name: '安装前风险确认' });
    fireEvent.click(within(dialog).getByRole('checkbox'));
    fireEvent.click(within(dialog).getByRole('button', { name: '确认安装' }));
    await screen.findByRole('button', { name: '已安装' });
    await act(async () => {});
    expect(api.installGitHubPlugin).toHaveBeenCalledWith({
      owner: 'owner', repo: 'community-plugin', branch: 'main',
    });
    const readsBeforeReturning = vi.mocked(api.getPlugins).mock.calls.length;
    fireEvent.click(screen.getByRole('tab', { name: '本地插件' }));
    const localPane = within(screen.getByRole('tabpanel', { name: '本地插件' }));
    expect(await localPane.findByRole('heading', { name: installedPlugin.name })).toBeVisible();
    expect(vi.mocked(api.getPlugins).mock.calls.length).toBeGreaterThan(readsBeforeReturning);
    expect(localPane.getByRole('button', { name: '运行' })).toBeEnabled();
    expect(localPane.queryByRole('heading', { name: '未发现插件' })).not.toBeInTheDocument();
    const searchesBeforeReturning = vi.mocked(api.discoverGitHubPlugins).mock.calls.length;
    fireEvent.click(screen.getByRole('tab', { name: '发现插件' }));
    expect(await screen.findByRole('button', { name: '已安装' })).toBeDisabled();
    await act(async () => {});
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(searchesBeforeReturning);
  });

  it('切换后保留本地编辑参数，仍可对所选视频执行插件', async () => {
    render(<PluginsPage />);
    await screen.findByRole('heading', { name: localPlugin.name });
    fireEvent.change(screen.getByDisplayValue('默认名称'), { target: { value: '精彩片段' } });
    fireEvent.change(screen.getByRole('spinbutton'), { target: { value: '0.8' } });
    await openDiscovery();
    fireEvent.click(screen.getByRole('tab', { name: '本地插件' }));
    expect(await screen.findByDisplayValue('精彩片段')).toBeVisible();
    expect(screen.getByRole('spinbutton')).toHaveValue(0.8);
    fireEvent.click(screen.getByRole('button', { name: '运行' }));
    await waitFor(() => expect(api.runPlugin).toHaveBeenCalledWith(
      localPlugin.name, video, { label: '精彩片段', threshold: 0.8 },
      expect.stringMatching(/^C:\/exports\/plugin-output\/local-plugin\/task-\d+$/),
    ));
    expect(await screen.findByText('[OK] 完成：生成 0 个标注')).toBeVisible();
  });
});
