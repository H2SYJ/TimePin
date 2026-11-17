import { invoke } from '@tauri-apps/api/core';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/lib/ipc';

beforeEach(() => {
  vi.mocked(invoke).mockReset();
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('GitHub discovery IPC contract', () => {
  it('discovers with camelCase args and no token', async () => {
    await api.discoverGitHubPlugins(2, 20);
    expect(invoke).toHaveBeenLastCalledWith('discover_github_plugins', { page: 2, perPage: 20 });
  });

  it('installs with camelCase args and no token', async () => {
    await api.installGitHubPlugin({ owner: 'o', repo: 'r', branch: 'main' });
    expect(invoke).toHaveBeenLastCalledWith('install_github_plugin', {
      owner: 'o',
      repo: 'r',
      branch: 'main',
    });
    const calls = vi.mocked(invoke).mock.calls;
    const payload = calls[calls.length - 1]?.[1] as Record<string, unknown>;
    expect('githubToken' in payload || 'github_token' in payload).toBe(false);
  });

  it('session commands use the new names with camelCase payload', async () => {
    await api.getGitHubSession();
    expect(invoke).toHaveBeenLastCalledWith('get_github_session');
    await api.configureGitHubToken('ghp_x', true);
    expect(invoke).toHaveBeenLastCalledWith('configure_github_token', {
      candidate: 'ghp_x',
      remember: true,
    });
    await api.clearGitHubToken();
    expect(invoke).toHaveBeenLastCalledWith('clear_github_token');
  });

  it('browser mode returns anonymous session and empty discover result without invoking', async () => {
    vi.stubGlobal('window', {});
    const session = await api.getGitHubSession();
    expect(session.identity).toBe('ANONYMOUS');
    const result = await api.discoverGitHubPlugins(1, 20);
    expect(result.schemaVersion).toBe('1.0.0');
    expect(result.plugins).toEqual([]);
    expect(result.rateLimit).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('browser mode rejects write operations as desktop-only', async () => {
    vi.stubGlobal('window', {});
    await expect(api.installGitHubPlugin({ owner: 'o', repo: 'r' })).rejects.toThrow(/桌面应用/);
    await expect(api.configureGitHubToken('x', false)).rejects.toThrow(/桌面应用/);
    await expect(api.clearGitHubToken()).rejects.toThrow(/桌面应用/);
    expect(invoke).not.toHaveBeenCalled();
  });
});
