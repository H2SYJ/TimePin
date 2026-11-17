import { StrictMode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { api } from '@/lib/ipc';
import type {
  DiscoveredPlugin,
  DiscoverPluginsResult,
  GitHubApiError,
  GitHubRateInfo,
  GitHubSession,
} from '@/types';
import { GitHubPluginDiscovery, __resetGitHubDiscoverySessionState } from './GitHubPluginDiscovery';

const anonymousSession: GitHubSession = {
  schemaVersion: '1.0.0',
  identity: 'ANONYMOUS',
  remembered: false,
  storageWarning: null,
  rateLimit: null,
};

const authenticatedSession: GitHubSession = {
  ...anonymousSession,
  identity: 'AUTHENTICATED',
  remembered: true,
};

function makePlugin(fullName = 'owner/alpha-plugin'): DiscoveredPlugin {
  const [owner = 'owner', name = 'alpha-plugin'] = fullName.split('/');
  return {
    repository: {
      name,
      fullName,
      description: '示例插件',
      htmlUrl: `https://github.com/${fullName}`,
      cloneUrl: `https://github.com/${fullName}.git`,
      stargazersCount: 10,
      forksCount: 2,
      updatedAt: '2026-01-01T00:00:00Z',
      language: 'Python',
      topics: ['time-pin-plugins'],
      owner: { login: owner, avatarUrl: '', htmlUrl: `https://github.com/${owner}` },
      defaultBranch: 'main',
    },
    manifest: {
      protocolVersion: '1.0.0',
      name: 'alpha-plugin',
      version: '1.0.0',
      description: 'desc',
      author: 'author',
      entry: 'main.py',
      runtime: 'python',
      capabilities: ['metadata'],
      parametersSchema: {},
      outputSchema: {},
    },
    manifestError: null,
    isInstalled: false,
    localPath: null,
    verification: 'community',
  };
}

function okResult(plugins = [makePlugin()], rateLimit: GitHubRateInfo | null = null): DiscoverPluginsResult {
  return { schemaVersion: '1.0.0', plugins, totalCount: plugins.length, rateLimit };
}

function ghError(code: GitHubApiError['code'], over: Partial<GitHubApiError> = {}): GitHubApiError {
  return {
    schemaVersion: '1.0.0',
    code,
    message: `error-${code}`,
    retryAt: null,
    action: code === 'AUTH_INVALID' ? 'REAUTHENTICATE'
      : ['PRIMARY_RATE_LIMIT', 'SECONDARY_RATE_LIMIT', 'RATE_LIMIT_UNKNOWN', 'RATE_WAIT'].includes(code) ? 'WAIT' : 'NONE',
    rateLimit: null,
    ...over,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  localStorage.clear();
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => true,
  }));
  __resetGitHubDiscoverySessionState();
  vi.spyOn(api, 'getGitHubSession').mockResolvedValue(anonymousSession);
  vi.spyOn(api, 'discoverGitHubPlugins').mockResolvedValue(okResult());
  vi.spyOn(api, 'installGitHubPlugin').mockResolvedValue('C:/plugins/alpha-plugin');
  vi.spyOn(api, 'configureGitHubToken').mockImplementation(async () => {
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    return authenticatedSession;
  });
  vi.spyOn(api, 'clearGitHubToken').mockImplementation(async () => {
    vi.mocked(api.getGitHubSession).mockResolvedValue(anonymousSession);
    return anonymousSession;
  });
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

// fireEvent wraps sync updates, but fire-and-forget microtasks (session refresh,
// deferred resolutions) settle outside act; flush them explicitly.
async function settled() {
  await act(async () => {});
}

async function loaded() {
  await waitFor(() => expect(api.discoverGitHubPlugins).toHaveBeenCalled());
  await settled();
}

describe('GitHub 插件发现', () => {
  it('默认匿名浏览：加载会话并搜索，不携带 token，删除遗留 localStorage', async () => {
    localStorage.setItem('github_token', 'ghp_leaky');
    render(<GitHubPluginDiscovery />);
    await loaded();
    expect(api.getGitHubSession).toHaveBeenCalled();
    expect(vi.mocked(api.discoverGitHubPlugins).mock.calls[0]).toEqual([1, 20]);
    expect(localStorage.getItem('github_token')).toBeNull();
    expect(screen.getByText('匿名')).toBeInTheDocument();
    expect(screen.queryByText(/剩余调用: 60/)).not.toBeInTheDocument();
    // 不再展示硬编码配额话术
    expect(screen.queryByText(/60 次\/小时|5000 次\/小时/)).not.toBeInTheDocument();
  });

  it('匿名额度低于 10 显示软警告，展示响应头中的剩余与重置时间', async () => {
    const resetAt = Date.now() + 30 * 60 * 1000;
    vi.mocked(api.discoverGitHubPlugins).mockResolvedValue(
      okResult([makePlugin()], { limit: 10, remaining: 5, resetAt, resource: 'search' }),
    );
    render(<GitHubPluginDiscovery />);
    await screen.findByText(/API 剩余调用: 5/);
    expect(screen.getByText(/额度即将用尽/)).toBeInTheDocument();
    expect(screen.getByText(/重置于:/)).toBeInTheDocument();
  });

  it('输入候选 token 不触发联网；只有提交才调用 configureGitHubToken', async () => {
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: /配置 Token/ }));
    const input = screen.getByLabelText(/Personal Access Token/);
    fireEvent.change(input, { target: { value: 'ghp_new_candidate' } });
    expect(api.configureGitHubToken).not.toHaveBeenCalled();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    await waitFor(() =>
      expect(api.configureGitHubToken).toHaveBeenCalledWith('ghp_new_candidate', false),
    );
  });

  it('验证失败时不重试原操作，错误贴近输入字段', async () => {
    vi.mocked(api.configureGitHubToken).mockRejectedValue(
      ghError('AUTH_INVALID', { message: 'token 无效', action: 'REAUTHENTICATE' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: /配置 Token/ }));
    fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'bad' } });
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    expect(await screen.findByText('token 无效')).toBeInTheDocument();
    expect(screen.getByLabelText(/Personal Access Token/)).toHaveAttribute('aria-invalid', 'true');
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
  });

  it('搜索限流后配置成功：只重试一次原搜索，输入被清空', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(
      ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000, action: 'WAIT' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    const buttons = screen.getAllByRole('button', { name: /配置 Token/ });
    fireEvent.click(buttons[buttons.length - 1]!);
    fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'ghp_ok' } });
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    await waitFor(() => expect(api.configureGitHubToken).toHaveBeenCalled());
    await waitFor(() => expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2));
    expect(vi.mocked(api.discoverGitHubPlugins).mock.calls[1]).toEqual([1, 20]);
    // 输入框随面板关闭而被移除，等价于已清空
    expect(screen.queryByLabelText(/Personal Access Token/)).not.toBeInTheDocument();
    await waitFor(() => expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2));
  });

  it('安装被 token 类错误拒绝后，配置成功只重试一次该安装', async () => {
    vi.mocked(api.installGitHubPlugin).mockRejectedValue(
      ghError('AUTH_INVALID', { action: 'REAUTHENTICATE' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: /安装/ }));
    // 先同意风险
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: '确认安装' }));
    await waitFor(() => expect(api.installGitHubPlugin).toHaveBeenCalledTimes(1));
    // 失败后自动打开 token 面板
    await screen.findByLabelText(/Personal Access Token/);
    vi.mocked(api.installGitHubPlugin).mockResolvedValue('C:/plugins/alpha-plugin');
    fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'ghp_ok' } });
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    await waitFor(() => expect(api.installGitHubPlugin).toHaveBeenCalledTimes(2));
    // 不重复触发同意弹窗
    expect(screen.queryByRole('dialog', { name: '安装前风险确认' })).not.toBeInTheDocument();
    await waitFor(() => expect(api.installGitHubPlugin).toHaveBeenCalledTimes(2));
  });

  it('取消 token 面板不再重试，保留已有搜索结果', async () => {
    vi.mocked(api.installGitHubPlugin).mockRejectedValue(
      ghError('AUTH_INVALID', { action: 'REAUTHENTICATE' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: /安装/ }));
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: '确认安装' }));
    await screen.findByLabelText(/Personal Access Token/);
    fireEvent.click(screen.getByRole('button', { name: '取消' }));
    expect(api.configureGitHubToken).not.toHaveBeenCalled();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
    expect(api.installGitHubPlugin).toHaveBeenCalledTimes(1);
    expect(screen.getByText('alpha-plugin')).toBeInTheDocument();
  });

  it('次级限流只提示等待，不引导配置 token', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(
      ghError('SECONDARY_RATE_LIMIT', { retryAt: Date.now() + 30000, action: 'WAIT' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    await screen.findByText(/请求被暂时限制/);
    expect(screen.queryByLabelText(/Personal Access Token/)).not.toBeInTheDocument();
  });

  it('401/AUTH_INVALID 错误展示重新配置引导', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(
      ghError('AUTH_INVALID', { message: 'token 无效', action: 'REAUTHENTICATE' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    expect(screen.getByText('token 无效')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: '重新配置 Token' }));
    await screen.findByLabelText(/Personal Access Token/);
  });

  it('会话 TOKEN_INVALID 时展示失效横幅并直接打开配置面板', async () => {
    vi.mocked(api.getGitHubSession).mockResolvedValue({
      ...anonymousSession,
      identity: 'TOKEN_INVALID',
      remembered: true,
    });
    render(<GitHubPluginDiscovery />);
    await loaded();
    expect((await screen.findAllByText(/Token 已失效/)).length).toBeGreaterThan(0);
    await screen.findByLabelText(/Personal Access Token/);
  });

  it('NOT_FOUND_OR_PRIVATE/NONE 提供 token 触点但不自动重试', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(
      ghError('NOT_FOUND_OR_PRIVATE', { action: 'NONE' }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    expect(screen.getByText(/仓库不存在或无权访问/)).toBeInTheDocument();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
    // 触点存在：错误内的按钮与工具栏按钮都可打开面板
    const buttons = screen.getAllByRole('button', { name: /配置 Token/ });
    expect(buttons.length).toBeGreaterThan(0);
  });

  it('凭据库失败提示仅内存保存', async () => {
    const warningSession = { ...authenticatedSession, remembered: false, storageWarning: 'keyring unavailable; token 仅保存在内存' };
    vi.mocked(api.configureGitHubToken).mockImplementation(async () => {
      vi.mocked(api.getGitHubSession).mockResolvedValue(warningSession);
      return warningSession;
    });
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: /配置 Token/ }));
    fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'ghp_ok' } });
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    expect(await screen.findByText(/token 仅保存在内存/)).toBeInTheDocument();
    expect(api.configureGitHubToken).toHaveBeenCalledWith('ghp_ok', false);
  });

  it('主限流横幅：配置/稍后/查看缓存三按钮，查看缓存保留结果且不会重复弹出', async () => {
    const first = deferred<DiscoverPluginsResult>();
    vi.mocked(api.discoverGitHubPlugins).mockReturnValue(first.promise);
    const { unmount } = render(<GitHubPluginDiscovery />);
    await waitFor(() => expect(api.discoverGitHubPlugins).toHaveBeenCalled());
    first.resolve(okResult());
    await screen.findByText('alpha-plugin');
    unmount();

    // 再次挂载：本次搜索触发主限流
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(
      ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }),
    );
    render(<GitHubPluginDiscovery />);
    const banner = await screen.findByRole('alert', { name: 'GitHub API 已达上限' });
    expect(within(banner).getByRole('button', { name: '配置 Token' })).toBeInTheDocument();
    expect(within(banner).getByRole('button', { name: '稍后' })).toBeInTheDocument();
    const bannerAlert = () => screen.queryByRole('alert', { name: 'GitHub API 已达上限' });
    const cacheButton = screen.getByRole('button', { name: '查看缓存' });
    expect(cacheButton).toBeEnabled();
    fireEvent.click(cacheButton);
    await screen.findByText(/内存缓存/);
    expect(bannerAlert()).not.toBeInTheDocument();

    // 会话单次：卸载重挂后即便再限流也不再弹横幅
    unmount();
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(
      ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }),
    );
    render(<GitHubPluginDiscovery />);
    await loaded();
    expect(bannerAlert()).not.toBeInTheDocument();
  });

  it('remember 勾选后 configure 收到 true', async () => {
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: /配置 Token/ }));
    fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'ghp_x' } });
    fireEvent.click(screen.getByRole('checkbox', { name: /记住此 token/ }));
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    await waitFor(() => expect(api.configureGitHubToken).toHaveBeenCalledWith('ghp_x', true));
  });
});

// confirmInstallation fires the async install flow whose finally block settles
// installing state + a fire-and-forget session refresh outside act; flush them.
async function confirmInstallation() {
  fireEvent.click(await screen.findByRole('button', { name: '安装' }));
  const dialog = screen.getByRole('dialog', { name: '安装前风险确认' });
  fireEvent.click(within(dialog).getByRole('checkbox'));
  fireEvent.click(within(dialog).getByRole('button', { name: '确认安装' }));
  await act(async () => {});
}

function openTokenPanel() {
  if (!screen.queryByLabelText(/Personal Access Token/)) {
    const buttons = screen.getAllByRole('button', { name: /配置 Token/ });
    fireEvent.click(buttons[buttons.length - 1]!);
  }
}

function submitToken(candidate = 'candidate') {
  fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: candidate } });
  fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
}

const invalidSession: GitHubSession = { ...anonymousSession, identity: 'TOKEN_INVALID' };

describe('GitHub 真实契约与并发回归', () => {
  it('匿名安装 PRIMARY_RATE_LIMIT/WAIT：验证成功前不重试，成功后只重试原安装且不再请求风险同意', async () => {
    const validation = deferred<GitHubSession>();
    vi.mocked(api.configureGitHubToken).mockReturnValue(validation.promise);
    vi.mocked(api.installGitHubPlugin).mockRejectedValueOnce(
      ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000, action: 'WAIT' }),
    );
    render(<GitHubPluginDiscovery />);
    await confirmInstallation();
    const banner = await screen.findByRole('alert', { name: 'GitHub API 已达上限' });
    expect(screen.getAllByRole('alert', { name: 'GitHub API 已达上限' })).toHaveLength(1);
    fireEvent.click(within(banner).getByRole('button', { name: '配置 Token' }));
    submitToken();
    expect(api.configureGitHubToken).toHaveBeenCalledWith('candidate', false);
    expect(api.installGitHubPlugin).toHaveBeenCalledTimes(1);
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    await act(async () => validation.resolve(authenticatedSession));
    await screen.findByRole('button', { name: '已安装' });
    await settled();
    expect(api.installGitHubPlugin).toHaveBeenCalledTimes(2);
    expect(vi.mocked(api.installGitHubPlugin).mock.calls).toEqual([
      [{ owner: 'owner', repo: 'alpha-plugin', branch: 'main' }],
      [{ owner: 'owner', repo: 'alpha-plugin', branch: 'main' }],
    ]);
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it.each([
    ['search', true], ['search', false], ['install', true], ['install', false],
  ] as const)('%s 完成（成功=%s）均刷新 session 并直接展开 TOKEN_INVALID 配置', async (operation, success) => {
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    const fallback = () => vi.mocked(api.getGitHubSession).mockResolvedValue(invalidSession);
    if (operation === 'search') {
      vi.mocked(api.discoverGitHubPlugins).mockImplementation(async () => {
        fallback();
        if (!success) throw ghError('NETWORK_ERROR');
        return okResult();
      });
    } else {
      vi.mocked(api.installGitHubPlugin).mockImplementation(async () => {
        fallback();
        if (!success) throw ghError('NOT_FOUND_OR_PRIVATE', { action: 'NONE' });
        return 'C:/plugins/alpha-plugin';
      });
    }
    render(<GitHubPluginDiscovery />);
    if (operation === 'install') await confirmInstallation();
    await screen.findByRole('alert', { name: 'Token 已失效' });
    await settled();
    expect(screen.getByLabelText(/Personal Access Token/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '清除 Token' })).not.toBeInTheDocument();
    expect(api.getGitHubSession).toHaveBeenCalledTimes(operation === 'search' ? 2 : 3);
  });

  it('401 撤销后的匿名回退再遇主限流，TOKEN_INVALID 仍可校验新候选并恢复原搜索', async () => {
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    vi.mocked(api.discoverGitHubPlugins).mockImplementationOnce(async () => {
      vi.mocked(api.getGitHubSession).mockResolvedValue(invalidSession);
      throw ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 });
    }).mockResolvedValueOnce(okResult());
    render(<GitHubPluginDiscovery />);
    await screen.findByRole('alert', { name: 'Token 已失效' });
    await settled();
    expect(screen.getByRole('button', { name: '验证并保存' })).toBeDisabled();
    fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'replacement' } });
    expect(screen.getByRole('button', { name: '验证并保存' })).toBeEnabled();
    fireEvent.click(screen.getByRole('button', { name: '验证并保存' }));
    await screen.findByText('alpha-plugin');
    await settled();
    expect(api.configureGitHubToken).toHaveBeenCalledWith('replacement', false);
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
    expect(screen.getByText('已认证')).toBeInTheDocument();
  });

  it.each(['success', 'primary-error'] as const)('配置新身份不复用匿名 Promise，旧 %s 不能覆盖新结果或重新锁定身份', async (outcome) => {
    const oldSearch = deferred<DiscoverPluginsResult>();
    vi.mocked(api.discoverGitHubPlugins).mockReturnValueOnce(oldSearch.promise)
      .mockResolvedValueOnce(okResult([makePlugin('owner/token-result')]));
    render(<GitHubPluginDiscovery />);
    await loaded();
    openTokenPanel();
    submitToken();
    await screen.findByText('token-result');
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
    await act(async () => {
      if (outcome === 'success') oldSearch.resolve(okResult([makePlugin('owner/old-result')]));
      else oldSearch.reject(ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }));
    });
    await settled();
    expect(screen.getByText('已认证')).toBeInTheDocument();
    expect(screen.getByText('token-result')).toBeInTheDocument();
    expect(screen.queryByText('old-result')).not.toBeInTheDocument();
    expect(screen.queryByRole('alert', { name: 'GitHub API 已达上限' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: '刷新' })).toBeEnabled();
  });

  it('清除凭据也使旧身份请求失效，并使用新代次发现', async () => {
    const oldSearch = deferred<DiscoverPluginsResult>();
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    vi.mocked(api.discoverGitHubPlugins).mockReturnValueOnce(oldSearch.promise)
      .mockResolvedValueOnce(okResult([makePlugin('owner/anonymous-result')]));
    render(<GitHubPluginDiscovery />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: '清除 Token' }));
    await screen.findByText('anonymous-result');
    await settled();
    await act(async () => oldSearch.reject(ghError('AUTH_INVALID')));
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
    expect(screen.getByText('匿名')).toBeInTheDocument();
    expect(screen.queryByLabelText(/Personal Access Token/)).not.toBeInTheDocument();
  });

  it('较晚到达的未安装发现快照不能撤销安装成功状态或缓存', async () => {
    const snapshot = deferred<DiscoverPluginsResult>();
    vi.mocked(api.discoverGitHubPlugins).mockResolvedValueOnce(okResult()).mockReturnValueOnce(snapshot.promise);
    const first = render(<GitHubPluginDiscovery />);
    await screen.findByText('alpha-plugin');
    fireEvent.click(screen.getByRole('button', { name: '刷新' }));
    await confirmInstallation();
    await screen.findByRole('button', { name: '已安装' });
    await settled();
    await act(async () => snapshot.resolve(okResult()));
    expect(screen.getByRole('button', { name: '已安装' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: '安装' })).not.toBeInTheDocument();
    first.unmount();
    vi.mocked(api.discoverGitHubPlugins).mockResolvedValue(okResult());
    render(<GitHubPluginDiscovery />);
    await waitFor(() => expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(3));
    expect(screen.getByRole('button', { name: '已安装' })).toBeDisabled();
  });

  it('初始化原文展示 storageWarning，不把系统凭据警告误称为仅内存', async () => {
    const warning = '系统凭据读取失败，请在下次启动前手动删除系统凭据。';
    vi.mocked(api.getGitHubSession).mockResolvedValue({ ...anonymousSession, storageWarning: warning });
    render(<GitHubPluginDiscovery />);
    expect(await screen.findByText(`凭据存储警告：${warning}`)).toBeInTheDocument();
    expect(screen.queryByText(/token 仅保存在内存/)).not.toBeInTheDocument();
  });

  it('清除凭据返回匿名但系统删除失败时，完整展示下次启动安全警告', async () => {
    const warning = '系统凭据删除失败；下次启动可能恢复，请手动删除系统凭据。';
    const next = { ...anonymousSession, storageWarning: warning };
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    vi.mocked(api.clearGitHubToken).mockImplementation(async () => {
      vi.mocked(api.getGitHubSession).mockResolvedValue(next);
      return next;
    });
    render(<GitHubPluginDiscovery />);
    await screen.findByText('alpha-plugin');
    fireEvent.click(screen.getByRole('button', { name: '清除 Token' }));
    expect(await screen.findByText(`凭据存储警告：${warning}`)).toBeInTheDocument();
    expect(screen.getByText('匿名')).toBeInTheDocument();
    expect(screen.queryByText(/token 仅保存在内存/)).not.toBeInTheDocument();
  });

  it('第2页失败后查看第1页缓存只切换视图，不触发 effect 联网；配置后仍重试原第2页', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockResolvedValueOnce({ ...okResult(), totalCount: 40 })
      .mockRejectedValueOnce(ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }))
      .mockResolvedValueOnce({ ...okResult([makePlugin('owner/page-two')]), totalCount: 40 });
    render(<GitHubPluginDiscovery />);
    await screen.findByText('alpha-plugin');
    fireEvent.click(screen.getByRole('button', { name: '下一页' }));
    const cache = await screen.findByRole('button', { name: '查看缓存' });
    fireEvent.click(cache);
    expect(screen.getByText(/内存缓存，未落盘/)).toBeInTheDocument();
    expect(screen.getByText('第 1 页 / 共 2 页')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
    openTokenPanel();
    submitToken();
    await screen.findByText('page-two');
    expect(vi.mocked(api.discoverGitHubPlugins).mock.calls).toEqual([[1, 20], [2, 20], [2, 20]]);
  });

  it.each(['SECONDARY_RATE_LIMIT', 'RATE_LIMIT_UNKNOWN', 'RATE_WAIT'] as const)(
    '搜索 %s（无 retryAt 停止状态）统一禁止刷新、分页、安装和候选验证', async (code) => {
      vi.mocked(api.discoverGitHubPlugins).mockResolvedValueOnce({ ...okResult(), totalCount: 40 })
        .mockRejectedValueOnce(ghError(code));
      render(<GitHubPluginDiscovery />);
      await screen.findByText('alpha-plugin');
      fireEvent.click(screen.getByRole('button', { name: '刷新' }));
      await screen.findByRole('alert', { name: '搜索失败' });
      await settled();
      expect(screen.getByRole('button', { name: '刷新' })).toBeDisabled();
      expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled();
      expect(screen.getByRole('button', { name: '安装' })).toBeDisabled();
      openTokenPanel();
      fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'candidate' } });
      expect(screen.getByRole('button', { name: '验证并保存' })).toBeDisabled();
      fireEvent.keyDown(screen.getByLabelText(/Personal Access Token/), { key: 'Enter' });
      fireEvent.click(screen.getByRole('button', { name: '下一页' }));
      fireEvent.click(screen.getByRole('button', { name: '安装' }));
      expect(api.configureGitHubToken).not.toHaveBeenCalled();
      expect(api.installGitHubPlugin).not.toHaveBeenCalled();
      expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
      expect(screen.queryByRole('alert', { name: 'GitHub API 已达上限' })).not.toBeInTheDocument();
    },
  );

  it.each(['SECONDARY_RATE_LIMIT', 'RATE_LIMIT_UNKNOWN'] as const)(
    '安装 %s 的 retryAt 同样门控全部联网入口', async (code) => {
      vi.mocked(api.discoverGitHubPlugins).mockResolvedValue({ ...okResult(), totalCount: 40 });
      vi.mocked(api.installGitHubPlugin).mockRejectedValue(ghError(code, { retryAt: Date.now() + 60000 }));
      render(<GitHubPluginDiscovery />);
      await confirmInstallation();
      await screen.findByRole('alert', { name: '安装失败' });
      await settled();
      expect(screen.getByRole('button', { name: /请求受限/ })).toBeDisabled();
      expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled();
      expect(screen.getByRole('button', { name: '安装' })).toBeDisabled();
      openTokenPanel();
      fireEvent.change(screen.getByLabelText(/Personal Access Token/), { target: { value: 'candidate' } });
      expect(screen.getByRole('button', { name: '验证并保存' })).toBeDisabled();
      expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
      expect(api.installGitHubPlugin).toHaveBeenCalledTimes(1);
    },
  );

  it('已认证主限流没有匿名候选验证例外，也不展示匿名横幅', async () => {
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    vi.mocked(api.discoverGitHubPlugins).mockResolvedValue({ ...okResult(), totalCount: 40 });
    vi.mocked(api.installGitHubPlugin).mockRejectedValue(ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }));
    render(<GitHubPluginDiscovery />);
    await confirmInstallation();
    await screen.findByRole('alert', { name: '安装失败' });
    await settled();
    expect(screen.getByRole('button', { name: /请求受限/ })).toBeDisabled();
    expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '安装' })).toBeDisabled();
    expect(screen.queryByRole('alert', { name: 'GitHub API 已达上限' })).not.toBeInTheDocument();
    expect(screen.queryByLabelText(/Personal Access Token/)).not.toBeInTheDocument();
  });

  it('等待到点只解禁入口，不自动重发；缓存模式仍禁止分页联网', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockResolvedValueOnce({ ...okResult(), totalCount: 40 })
      .mockRejectedValueOnce(ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }));
    render(<GitHubPluginDiscovery />);
    await screen.findByText('alpha-plugin');
    vi.useFakeTimers();
    try {
      fireEvent.click(screen.getByRole('button', { name: '下一页' }));
      await act(async () => {});
      fireEvent.click(screen.getByRole('button', { name: '查看缓存' }));
      await act(async () => { vi.advanceTimersByTime(60001); });
      expect(screen.getByRole('button', { name: '刷新' })).toBeEnabled();
      expect(screen.getByRole('button', { name: '下一页' })).toBeDisabled();
      expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it('无效 token 修正后可以立即再次提交，并同步防止进行中的 Enter/点击双提交', async () => {
    const validation = deferred<GitHubSession>();
    vi.mocked(api.configureGitHubToken).mockRejectedValueOnce(ghError('AUTH_INVALID', { message: 'invalid candidate' }))
      .mockReturnValueOnce(validation.promise);
    render(<GitHubPluginDiscovery />);
    await screen.findByText('alpha-plugin');
    openTokenPanel();
    submitToken('bad');
    await screen.findByText('invalid candidate');
    expect(screen.getByRole('button', { name: '验证并保存' })).toBeEnabled();
    submitToken('corrected');
    fireEvent.keyDown(screen.getByLabelText(/Personal Access Token/), { key: 'Enter' });
    expect(api.configureGitHubToken).toHaveBeenCalledTimes(2);
    expect(api.configureGitHubToken).toHaveBeenLastCalledWith('corrected', false);
    vi.mocked(api.getGitHubSession).mockResolvedValue(authenticatedSession);
    await act(async () => validation.resolve(authenticatedSession));
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2);
  });

  it('新 session 查询先到达后，旧 session Promise 不得恢复已撤销身份', async () => {
    const oldSession = deferred<GitHubSession>();
    vi.mocked(api.getGitHubSession).mockResolvedValueOnce(authenticatedSession)
      .mockReturnValueOnce(oldSession.promise).mockResolvedValue(invalidSession);
    render(<GitHubPluginDiscovery />);
    await screen.findByText('alpha-plugin');
    await waitFor(() => expect(api.getGitHubSession).toHaveBeenCalledTimes(2));
    await settled();
    await confirmInstallation();
    await screen.findByRole('alert', { name: 'Token 已失效' });
    await settled();
    await act(async () => oldSession.resolve(authenticatedSession));
    expect(screen.queryByText('已认证')).not.toBeInTheDocument();
    expect(screen.getByLabelText(/Personal Access Token/)).toBeInTheDocument();
  });

  it('初始化 session 查询晚于配置成功，不覆盖身份也不补发旧初始化搜索', async () => {
    const oldSession = deferred<GitHubSession>();
    vi.mocked(api.getGitHubSession).mockReturnValueOnce(oldSession.promise);
    render(<GitHubPluginDiscovery />);
    openTokenPanel();
    submitToken();
    await screen.findByText('alpha-plugin');
    await act(async () => oldSession.resolve(anonymousSession));
    expect(screen.getByText('已认证')).toBeInTheDocument();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
  });

  it('安装 404/NONE 不自动配置或重试安装', async () => {
    vi.mocked(api.installGitHubPlugin).mockRejectedValue(ghError('NOT_FOUND_OR_PRIVATE', { action: 'NONE' }));
    render(<GitHubPluginDiscovery />);
    await confirmInstallation();
    await screen.findByRole('alert', { name: '安装失败' });
    await settled();
    expect(screen.queryByLabelText(/Personal Access Token/)).not.toBeInTheDocument();
    openTokenPanel();
    submitToken();
    await waitFor(() => expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(2));
    expect(api.installGitHubPlugin).toHaveBeenCalledTimes(1);
  });

  it('主限流横幅即使未点击确认就卸载，本应用会话也不重复弹出', async () => {
    vi.mocked(api.discoverGitHubPlugins).mockRejectedValue(ghError('PRIMARY_RATE_LIMIT', { retryAt: Date.now() + 60000 }));
    const first = render(<GitHubPluginDiscovery />);
    await screen.findByRole('alert', { name: 'GitHub API 已达上限' });
    first.unmount();
    render(<GitHubPluginDiscovery />);
    await screen.findByRole('alert', { name: '搜索失败' });
    expect(screen.queryByRole('alert', { name: 'GitHub API 已达上限' })).not.toBeInTheDocument();
  });

  it('StrictMode 同代次初始化搜索仍然只执行一次', async () => {
    const search = deferred<DiscoverPluginsResult>();
    vi.mocked(api.discoverGitHubPlugins).mockReturnValue(search.promise);
    render(<StrictMode><GitHubPluginDiscovery /></StrictMode>);
    await loaded();
    expect(api.discoverGitHubPlugins).toHaveBeenCalledTimes(1);
    await act(async () => search.resolve(okResult()));
    expect(screen.getByText('alpha-plugin')).toBeInTheDocument();
  });
});
