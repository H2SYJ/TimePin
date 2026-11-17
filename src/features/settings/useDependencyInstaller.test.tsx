import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { DependencyInstallProgress, InstalledDependencies } from '@/types';
import { useDependencyInstaller } from './useDependencyInstaller';

const defaults = useAppStore.getState().settings;
const installed: InstalledDependencies = {
  schemaVersion: '1.0.0', ffmpegPath: '/app/ffmpeg', ffprobePath: '/app/ffprobe',
  report: { schemaVersion: '1.0.0', dependencies: [] },
};
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

beforeEach(() => {
  useAppStore.setState({ settings: defaults });
  vi.spyOn(api, 'onDependencyInstallProgress').mockResolvedValue(() => {});
  vi.spyOn(api, 'installDependencies').mockResolvedValue(installed);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('dependency installer subscription lifecycle', () => {
  it('awaits listener registration before invoking installation and rejects overlapping attempts', async () => {
    const registration = deferred<() => void>();
    const pending = deferred<InstalledDependencies>();
    const unlisten = vi.fn();
    vi.mocked(api.onDependencyInstallProgress).mockReturnValue(registration.promise);
    vi.mocked(api.installDependencies).mockReturnValue(pending.promise);
    const onInstalled = vi.fn();
    const { result } = renderHook(() => useDependencyInstaller(onInstalled));
    let completion!: Promise<void>;
    act(() => { completion = result.current.install(); });
    expect(result.current.installing).toBe(true);
    expect(api.installDependencies).not.toHaveBeenCalled();
    await act(async () => { await result.current.install(); });
    expect(api.onDependencyInstallProgress).toHaveBeenCalledOnce();
    await act(async () => { registration.resolve(unlisten); });
    expect(api.installDependencies).toHaveBeenCalledOnce();
    await act(async () => { pending.resolve(installed); await completion; });
    expect(onInstalled).toHaveBeenCalledWith(installed);
    expect(unlisten).toHaveBeenCalledOnce();
    expect(result.current.installing).toBe(false);
  });

  it('immediately releases a late listener registration after unmount without invoking install', async () => {
    const registration = deferred<() => void>();
    const unlisten = vi.fn();
    vi.mocked(api.onDependencyInstallProgress).mockReturnValue(registration.promise);
    const { result, unmount } = renderHook(() => useDependencyInstaller(vi.fn()));
    let completion!: Promise<void>;
    act(() => { completion = result.current.install(); });
    unmount();
    await act(async () => { registration.resolve(unlisten); await completion; });
    expect(unlisten).toHaveBeenCalledOnce();
    expect(api.installDependencies).not.toHaveBeenCalled();
  });

  it('releases an active listener on unmount, ignores late events and merges persisted paths after completion', async () => {
    const pending = deferred<InstalledDependencies>();
    const unlisten = vi.fn();
    const onInstalled = vi.fn();
    let handler: ((event: DependencyInstallProgress) => void) | undefined;
    vi.mocked(api.onDependencyInstallProgress).mockImplementation(async (callback) => { handler = callback; return unlisten; });
    vi.mocked(api.installDependencies).mockReturnValue(pending.promise);
    const { result, unmount } = renderHook(() => useDependencyInstaller(onInstalled));
    let completion!: Promise<void>;
    act(() => { completion = result.current.install(); });
    await waitFor(() => expect(api.installDependencies).toHaveBeenCalledOnce());
    const taskId = vi.mocked(api.installDependencies).mock.calls[0]![0];
    unmount();
    expect(unlisten).toHaveBeenCalledOnce();
    act(() => handler?.({ schemaVersion: '1.0.0', taskId, stage: 'download', progress: 0.5, message: 'late event', error: null }));
    useAppStore.setState({ settings: { ...defaults, language: 'en-US' } });
    await act(async () => { pending.resolve(installed); await completion; });
    expect(onInstalled).not.toHaveBeenCalled();
    expect(unlisten).toHaveBeenCalledOnce();
    expect(useAppStore.getState().settings).toEqual({ ...defaults, language: 'en-US', ffmpegPath: installed.ffmpegPath, ffprobePath: installed.ffprobePath });
  });

  it('does not invoke installation if listener registration fails, and permits retry', async () => {
    const onInstalled = vi.fn();
    vi.mocked(api.onDependencyInstallProgress).mockRejectedValueOnce(new Error('listener unavailable'));
    const { result } = renderHook(() => useDependencyInstaller(onInstalled));
    await act(async () => { await result.current.install(); });
    expect(result.current.error).toBe('listener unavailable');
    expect(result.current.installing).toBe(false);
    expect(api.installDependencies).not.toHaveBeenCalled();
    await act(async () => { await result.current.install(); });
    expect(onInstalled).toHaveBeenCalledWith(installed);
    expect(result.current.error).toBeNull();
  });
});
