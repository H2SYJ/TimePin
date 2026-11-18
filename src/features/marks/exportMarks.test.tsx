import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { api, isTauri } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import { useMarksStore } from '@/stores/marks';
import { useUpdateStore } from '@/stores/updater';
import type { Mark, MarksDocument, VideoInfo } from '@/types';
import { exportableMarks, isExportableInterval, validSelectedIds } from './exportMarks';
import { MarksPage } from './MarksPage';

// Wrap isTauri so tests can simulate the browser guard without touching the
// non-configurable __TAURI_INTERNALS__ property defined in test setup.
vi.mock('@/lib/ipc', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@/lib/ipc')>();
  return { ...actual, isTauri: vi.fn(() => actual.isTauri()) };
});
const mockedIsTauri = vi.mocked(isTauri);

let sequence = 0;
let video: VideoInfo;
const defaults = useAppStore.getState().settings;

const document = (videoId: string, marks: Mark[] = []): MarksDocument => ({ schemaVersion: '1.0.0', videoId, marks });

const baseMark: Mark = {
  id: 'mark', startTime: 10, endTime: 20, label: '有效区间', color: null, confidence: 0.5,
  sourcePlugin: 'test', thumbnailPath: 'ev.png', metadata: { key: 'value' },
};

function point(id: string, startTime: number): Mark {
  return { ...baseMark, id, startTime, endTime: null, label: `点${id}` };
}
function interval(id: string, startTime: number, endTime: number, label = `区间${id}`): Mark {
  return { ...baseMark, id, startTime, endTime, label };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function readyPlayer(time = 0, duration = 100): HTMLVideoElement {
  const player = screen.getByLabelText(`${video.name} 标注播放器`) as HTMLVideoElement;
  Object.defineProperty(player, 'duration', { configurable: true, value: duration });
  player.currentTime = time;
  fireEvent.loadedMetadata(player);
  return player;
}

async function loaded() {
  await waitFor(() => expect(useMarksStore.getState().entries[video.id]?.loadStatus).toBe('ready'));
}

function seedMarks(marks: Mark[]) {
  vi.mocked(api.getMarks).mockResolvedValueOnce(document(video.id, marks));
}

function exportButton() {
  // The button keeps a stable aria-label while its visible text shows busy state.
  return screen.getByRole('button', { name: '导出所选区间' });
}

beforeEach(() => {
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    matches: false, media: query, onchange: null,
    addListener: () => {}, removeListener: () => {},
    addEventListener: () => {}, removeEventListener: () => {}, dispatchEvent: () => true,
  }));
  sequence += 1;
  video = {
    id: `exp-video-${sequence}`, name: `导出 ${sequence}.mp4`, path: `C:/exp-${sequence}.mp4`, sourcePath: 'C:/source.mp4',
    duration: 100, width: 1920, height: 1080, fps: 30, codec: 'h264', fileSize: 100,
    importedAt: '2026-01-01', thumbnailPath: null, sidecarTxtPath: null, sidecarCsvPath: null, sidecarXmlPath: null, tags: [], metadata: {},
  };
  useAppStore.setState({ videos: [video], selectedVideoId: video.id, page: 'marks', settings: { ...defaults } });
  useMarksStore.setState({ entries: {} });
  useUpdateStore.setState({ busyWorkKinds: [], workTokens: [] });
  vi.spyOn(api, 'getMarks').mockImplementation(async (id) => document(id));
  vi.spyOn(api, 'saveMarks').mockImplementation(async (id, marks) => document(id, marks));
  vi.spyOn(api, 'pickDirectory').mockResolvedValue('C:/out');
  vi.spyOn(api, 'exportSegments').mockResolvedValue(['C:/out/a.mp4', 'C:/out/b.mp4']);
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('exportMarks helpers', () => {
  it('accepts only finite nonnegative intervals strictly inside duration', () => {
    expect(isExportableInterval(interval('a', 0, 100), 100)).toBe(true);
    expect(isExportableInterval(point('p', 10), 100)).toBe(false);
    expect(isExportableInterval(interval('s', -1, 10), 100)).toBe(false);
    expect(isExportableInterval(interval('e', 10, 10), 100)).toBe(false);
    expect(isExportableInterval(interval('r', 20, 10), 100)).toBe(false);
    expect(isExportableInterval(interval('o', 10, 101), 100)).toBe(false);
    expect(isExportableInterval(interval('n', Number.NaN, 10), 100)).toBe(false);
    expect(isExportableInterval(interval('i', 0, Number.POSITIVE_INFINITY), 100)).toBe(false);
    expect(isExportableInterval(interval('z', 0, 10), 0)).toBe(false);
    expect(exportableMarks([point('p', 1), interval('a', 1, 2), interval('o', 90, 200)], 100).map((m) => m.id)).toEqual(['a']);
    expect(validSelectedIds(['a', 'p', 'x'], [interval('a', 1, 2), point('p', 1)], 100)).toEqual(['a']);
  });
});

describe('segment export panel', () => {
  it('marks points non-exportable, requires explicit selection and sends the exact payload', async () => {
    seedMarks([
      point('p1', 5),
      interval('good', 10, 20, '片段A'),
      interval('bad', 90, 120, '超长'),
      interval('zero', 30, 30, '零长'),
    ]);
    render(<MarksPage />);
    await loaded();
    const player = readyPlayer(7);

    // Time point and invalid intervals are visible but not selectable.
    const pointCheckbox = screen.getByRole('checkbox', { name: '导出 点p1' });
    const badCheckbox = screen.getByRole('checkbox', { name: '导出 超长' });
    const zeroCheckbox = screen.getByRole('checkbox', { name: '导出 零长' });
    expect(pointCheckbox).toBeDisabled();
    expect(badCheckbox).toBeDisabled();
    expect(zeroCheckbox).toBeDisabled();
    expect(screen.getByText('· 不可导出（时间点）')).toBeInTheDocument();
    expect(screen.getAllByText('· 不可导出（无效或超出时长）')).toHaveLength(2);
    expect(screen.getByText((_, el) => el?.textContent === '已选 0/1 个区间 · 流复制（-c copy），不重编码、不修改原视频')).toBeInTheDocument();

    // No selection → export disabled.
    expect(exportButton()).toBeDisabled();

    const checkbox = screen.getByRole('checkbox', { name: '导出 片段A' });
    // Toggling the checkbox must not seek the player.
    fireEvent.click(checkbox);
    expect(player.currentTime).toBe(7);
    expect(exportButton()).not.toBeDisabled();
    expect(screen.getByText((_, el) => el?.textContent === '已选 1/1 个区间 · 流复制（-c copy），不重编码、不修改原视频')).toBeInTheDocument();
    // Selection is React state only — nothing persisted.
    expect(api.saveMarks).not.toHaveBeenCalled();

    fireEvent.click(exportButton());
    await waitFor(() => expect(api.pickDirectory).toHaveBeenCalled());
    expect(api.pickDirectory).toHaveBeenCalledWith('选择导出目录');
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(1));
    const call = vi.mocked(api.exportSegments).mock.calls[0]!;
    expect(call[0]).toBe(video.path);
    expect(call[2]).toBe('C:/out');
    expect(call[3]).toBe('streamCopy');
    expect(call[1]).toHaveLength(1);
    expect(call[1][0]).toMatchObject({ id: 'good', startTime: 10, endTime: 20, confidence: 0.5, sourcePlugin: 'test', thumbnailPath: 'ev.png', metadata: { key: 'value' } });

    // Actual returned paths shown; selection retained after success.
    expect(await screen.findByText('导出完成')).toBeInTheDocument();
    expect(screen.getByText('· C:/out/a.mp4')).toBeInTheDocument();
    expect(screen.getByText('· C:/out/b.mp4')).toBeInTheDocument();
    expect(checkbox).toBeChecked();
  });

  it('honors a live defaultExportMode change and blocks frameAccurate without fallback', async () => {
    useAppStore.setState({ settings: { ...defaults, defaultExportMode: 'frameAccurate' } });
    seedMarks([interval('good', 10, 20)]);
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 区间good' }));
    fireEvent.click(exportButton());
    expect(await screen.findByRole('alert')).toHaveTextContent(/帧精确.*尚未支持|尚未支持.*帧精确/);
    expect(api.pickDirectory).not.toHaveBeenCalled();
    expect(api.exportSegments).not.toHaveBeenCalled();
    // Selection retained for retry after switching mode.
    expect(screen.getByRole('checkbox', { name: '导出 区间good' })).toBeChecked();

    act(() => { useAppStore.setState({ settings: { ...defaults, defaultExportMode: 'streamCopy' } }); });
    fireEvent.click(exportButton());
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(1));
    expect(vi.mocked(api.exportSegments).mock.calls[0]![3]).toBe('streamCopy');
  });

  it('drops deleted marks and resets stale state across video switches', async () => {
    seedMarks([interval('a', 1, 2, '第一'), interval('b', 3, 4, '第二')]);
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 第一' }));
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 第二' }));
    expect(screen.getByText((_, el) => el?.textContent === '已选 2/2 个区间 · 流复制（-c copy），不重编码、不修改原视频')).toBeInTheDocument();
    // Deleting a selected mark removes it from the selection automatically.
    fireEvent.click(screen.getByRole('button', { name: '删除 第一' }));
    await waitFor(() => expect(screen.getByText((_, el) => el?.textContent === '已选 1/1 个区间 · 流复制（-c copy），不重编码、不修改原视频')).toBeInTheDocument());
    expect(api.exportSegments).not.toHaveBeenCalled();
  });

  it('treats directory cancel as harmless and keeps the selection', async () => {
    vi.mocked(api.pickDirectory).mockResolvedValueOnce(null);
    seedMarks([interval('a', 1, 2, '保留')]);
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 保留' }));
    fireEvent.click(exportButton());
    await waitFor(() => expect(exportButton()).not.toBeDisabled());
    expect(api.exportSegments).not.toHaveBeenCalled();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByRole('checkbox', { name: '导出 保留' })).toBeChecked();
    // Immediate retry works after cancel.
    fireEvent.click(exportButton());
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(1));
  });

  it('shows a readable export error and retries with the retained selection', async () => {
    vi.mocked(api.exportSegments).mockRejectedValueOnce(new Error('ffmpeg 退出码 1'));
    seedMarks([interval('a', 1, 2, '重试')]);
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 重试' }));
    fireEvent.click(exportButton());
    expect(await screen.findByRole('alert')).toHaveTextContent(/导出失败.*ffmpeg 退出码 1/);
    expect(screen.getByRole('checkbox', { name: '导出 重试' })).toBeChecked();
    fireEvent.click(exportButton());
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(2));
    expect(await screen.findByText('导出完成')).toBeInTheDocument();
  });

  it('prevents duplicate submissions while the picker is pending and during export', async () => {
    const picker = deferred<string | null>();
    vi.mocked(api.pickDirectory).mockImplementationOnce(() => picker.promise);
    const exporting = deferred<string[]>();
    vi.mocked(api.exportSegments).mockImplementationOnce(() => exporting.promise);
    seedMarks([interval('a', 1, 2, '锁')]);
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 锁' }));
    fireEvent.click(exportButton());
    // Deferred picker: still busy, second click must not open another dialog.
    expect(screen.getAllByText('等待选择导出目录…').length).toBeGreaterThan(0);
    fireEvent.click(exportButton());
    expect(api.pickDirectory).toHaveBeenCalledTimes(1);
    await act(async () => { picker.resolve('C:/out'); });
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(1));
    // In-flight export: still busy, second click must not invoke again.
    expect(screen.getAllByText('正在导出片段…').length).toBeGreaterThan(0);
    fireEvent.click(exportButton());
    expect(api.exportSegments).toHaveBeenCalledTimes(1);
    await act(async () => { exporting.resolve([]); });
    await waitFor(() => expect(exportButton()).not.toBeDisabled());
    expect(api.exportSegments).toHaveBeenCalledTimes(1);
  });

  it('holds the updater export token during IPC and releases it after unmount', async () => {
    const exporting = deferred<string[]>();
    vi.mocked(api.exportSegments).mockImplementationOnce(() => exporting.promise);
    seedMarks([interval('a', 1, 2, '令牌')]);
    const view = render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 令牌' }));
    fireEvent.click(exportButton());
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(1));
    // Token taken before the real IPC and held while export runs.
    expect(useUpdateStore.getState().busyWorkKinds).toContain('export');
    view.unmount();
    await act(async () => { exporting.resolve(['C:/out/late.mp4']); });
    // Released even though the component is gone.
    expect(useUpdateStore.getState().busyWorkKinds).not.toContain('export');
    expect(useUpdateStore.getState().workTokens).toHaveLength(0);
  });

  it('suppresses stale export completion after switching to another video', async () => {
    const exporting = deferred<string[]>();
    vi.mocked(api.exportSegments).mockImplementationOnce(() => exporting.promise);
    seedMarks([interval('a', 1, 2, '旧视频区间')]);
    const second: VideoInfo = { ...video, id: `${video.id}-b`, name: '新视频.mp4', path: 'C:/new.mp4' };
    useAppStore.setState({ videos: [video, second] });
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 旧视频区间' }));
    fireEvent.click(exportButton());
    await waitFor(() => expect(api.exportSegments).toHaveBeenCalledTimes(1));

    // Switch video mid-export: workspace remounts, old success must not surface.
    act(() => { useAppStore.getState().selectVideo(second.id); });
    video = second;
    await loaded();
    readyPlayer();
    await act(async () => { exporting.resolve(['C:/out/old.mp4']); });
    expect(screen.queryByText('导出完成')).not.toBeInTheDocument();
    expect(screen.queryByText('· C:/out/old.mp4')).not.toBeInTheDocument();
    expect(useUpdateStore.getState().busyWorkKinds).not.toContain('export');
    // New video's export button reflects its own (empty) selection.
    expect(screen.getByText((_, el) => el?.textContent?.startsWith('已选 0/0 个区间') ?? false)).toBeInTheDocument();
  });

  it('blocks browser-only export with an explicit message and no IPC', async () => {
    mockedIsTauri.mockReturnValue(false);
    seedMarks([interval('a', 1, 2, '浏览器')]);
    render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('checkbox', { name: '导出 浏览器' }));
    fireEvent.click(exportButton());
    expect(await screen.findByRole('alert')).toHaveTextContent(/需要在 TimePin 桌面应用中运行/);
    expect(api.exportSegments).not.toHaveBeenCalled();
    expect(api.pickDirectory).not.toHaveBeenCalled();
  });
});
