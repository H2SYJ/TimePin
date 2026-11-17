import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { api } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import { useMarksStore, type MarksEntry } from '@/stores/marks';
import type { Mark, MarksDocument, VideoInfo } from '@/types';
import { MarksPage } from './MarksPage';

let sequence = 0;
let video: VideoInfo;
const document = (videoId: string, marks: Mark[] = []): MarksDocument => ({ schemaVersion: '1.0.0', videoId, marks });
const pluginMark: Mark = {
  id: 'plugin', startTime: 15.25, endTime: 20.5, label: '插件标注', color: '#123456', confidence: 0.91,
  sourcePlugin: 'example', thumbnailPath: 'evidence/frame.png', metadata: { original: 'preserve' },
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((yes) => { resolve = yes; });
  return { promise, resolve };
}

function marksEntry(videoId: string): MarksEntry {
  const value = useMarksStore.getState().entries[videoId];
  if (!value) throw new Error(`missing marks entry for ${videoId}`);
  return value;
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

async function saved() {
  await waitFor(() => expect(useMarksStore.getState().entries[video.id]?.saveStatus).toBe('idle'));
}

beforeEach(() => {
  vi.spyOn(window, 'matchMedia').mockImplementation((query) => ({
    matches: false, media: query, onchange: null,
    addListener: () => {}, removeListener: () => {},
    addEventListener: () => {}, removeEventListener: () => {}, dispatchEvent: () => true,
  }));
  sequence += 1;
  video = {
    id: `ui-video-${sequence}`, name: `录制 ${sequence}.mp4`, path: `C:/video-${sequence}.mp4`, sourcePath: 'C:/source.mp4',
    duration: 100, width: 1920, height: 1080, fps: 30, codec: 'h264', fileSize: 100,
    importedAt: '2026-01-01', thumbnailPath: null, sidecarTxtPath: null, sidecarCsvPath: null, sidecarXmlPath: null, tags: [], metadata: {},
  };
  useAppStore.setState({ videos: [video], selectedVideoId: video.id, page: 'marks' });
  useMarksStore.setState({ entries: {} });
  vi.spyOn(api, 'getMarks').mockImplementation(async (id) => document(id));
  vi.spyOn(api, 'saveMarks').mockImplementation(async (id, marks) => document(id, marks));
});

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

describe('manual marks workspace', () => {
  it('offers a media-library entry with no video and sends no mark writes', () => {
    useAppStore.setState({ videos: [], selectedVideoId: null });
    render(<MarksPage />);
    expect(screen.getByText('还没有可标注的视频')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'm' });
    expect(api.getMarks).not.toHaveBeenCalled();
    expect(api.saveMarks).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '前往视频库' }));
    expect(useAppStore.getState().page).toBe('library');
  });

  it('waits for metadata, uses exact currentTime for M/R, and normalizes reverse intervals', async () => {
    render(<MarksPage />);
    await loaded();
    expect(screen.getByRole('button', { name: '添加时间点（M）' })).toBeDisabled();
    fireEvent.keyDown(window, { key: 'm' });
    expect(api.saveMarks).not.toHaveBeenCalled();
    const earlyPlayer = screen.getByLabelText(`${video.name} 标注播放器`) as HTMLVideoElement;
    Object.defineProperty(earlyPlayer, 'duration', { configurable: true, value: 100 });
    fireEvent.durationChange(earlyPlayer);
    expect(screen.getByRole('button', { name: '添加时间点（M）' })).toBeDisabled();
    const player = readyPlayer(12.345678);
    expect(player).toHaveAttribute('controls');
    expect(player).toHaveAttribute('src', video.path);
    // No timeupdate: command must read directly from the media element, not stale UI time.
    player.currentTime = 13.987654;
    fireEvent.keyDown(window, { key: 'm' });
    await saved();
    expect(marksEntry(video.id).marks[0]).toMatchObject({ startTime: 13.987654, endTime: null });
    player.currentTime = 40.987654;
    fireEvent.keyDown(window, { key: 'r' });
    player.currentTime = 20.123456;
    fireEvent.keyDown(window, { key: 'r' });
    await saved();
    expect(marksEntry(video.id).marks[1]).toMatchObject({ startTime: 20.123456, endTime: 40.987654 });
    expect(screen.getByRole('button', { name: '开始区间（R）' })).toBeInTheDocument();
  });

  it('rejects equal interval endpoints and Escape cancels without writing', async () => {
    render(<MarksPage />);
    await loaded();
    const player = readyPlayer(8.123);
    fireEvent.keyDown(window, { key: 'r' });
    fireEvent.keyDown(window, { key: 'r' });
    expect(screen.getByText('区间首尾不能相同，请移动播放位置后再次按 R')).toBeInTheDocument();
    expect(api.saveMarks).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(screen.getByText('已取消未完成区间')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: '取消区间（Esc）' })).not.toBeInTheDocument();
    player.currentTime = 9;
    fireEvent.keyDown(window, { key: 'r' });
    expect(api.saveMarks).not.toHaveBeenCalled();
  });

  it('ignores input/modifier/repeated keys, while toolbar buttons still add marks', async () => {
    render(<MarksPage />);
    await loaded();
    readyPlayer(1.234);
    fireEvent.keyDown(screen.getByLabelText('当前视频'), { key: 'm' });
    fireEvent.keyDown(screen.getByLabelText('播放位置'), { key: 'r' });
    fireEvent.keyDown(window, { key: 'm', repeat: true });
    fireEvent.keyDown(window, { key: 'm', ctrlKey: true });
    fireEvent.keyDown(window, { key: 'r', shiftKey: true });
    expect(api.saveMarks).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '添加时间点（M）' }));
    await saved();
    fireEvent.click(screen.getByRole('button', { name: '编辑 标记点' }));
    const input = screen.getByRole('textbox', { name: '标注名称' });
    fireEvent.keyDown(input, { key: 'm' });
    fireEvent.keyDown(input, { key: 'r' });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(api.saveMarks).toHaveBeenCalledTimes(1);
  });

  it('synchronizes native playback, slider, timeline and list seeks', async () => {
    vi.mocked(api.getMarks).mockResolvedValueOnce(document(video.id, [pluginMark, { ...pluginMark, id: 'point', label: '时间点', startTime: 35.125, endTime: null }]));
    render(<MarksPage />);
    await loaded();
    const player = readyPlayer(10);
    const slider = screen.getByRole('slider', { name: '播放位置' });
    player.currentTime = 11.375;
    fireEvent.timeUpdate(player);
    expect(slider).toHaveValue('11.375');
    fireEvent.change(slider, { target: { value: '30.125' } });
    expect(player.currentTime).toBe(30.125);
    fireEvent.click(screen.getByRole('button', { name: '跳转到 插件标注' }));
    expect(player.currentTime).toBe(15.25);
    fireEvent.click(screen.getByRole('button', { name: /^时间轴：时间点/ }));
    expect(player.currentTime).toBe(35.125);
    expect(slider).toHaveValue('35.125');
    expect(screen.getByRole('button', { name: /^时间轴：插件标注/ })).toHaveStyle({ width: '5.25%' });
    expect(api.saveMarks).not.toHaveBeenCalled();
  });

  it('renames/deletes with persisted writes, keeps plugin metadata and cached reload', async () => {
    vi.mocked(api.getMarks).mockResolvedValueOnce(document(video.id, [pluginMark]));
    const view = render(<MarksPage />);
    await loaded();
    readyPlayer();
    fireEvent.click(screen.getByRole('button', { name: '编辑 插件标注' }));
    const input = screen.getByRole('textbox', { name: '标注名称' });
    fireEvent.change(input, { target: { value: ' 新名称 ' } });
    fireEvent.click(screen.getByRole('button', { name: '保存名称' }));
    await saved();
    expect(api.saveMarks).toHaveBeenLastCalledWith(video.id, [{ ...pluginMark, label: '新名称' }]);
    view.unmount();
    render(<MarksPage />);
    expect(await screen.findByRole('button', { name: '编辑 新名称' })).toBeInTheDocument();
    expect(api.getMarks).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole('button', { name: '删除 新名称' }));
    await saved();
    expect(api.saveMarks).toHaveBeenLastCalledWith(video.id, []);
    expect(screen.queryByRole('list', { name: '标注列表' })).not.toBeInTheDocument();
  });

  it('locks failed initial loads, then retries; save errors retain a draft and retry visibly', async () => {
    vi.mocked(api.getMarks).mockRejectedValueOnce(new Error('corrupt marks'));
    render(<MarksPage />);
    readyPlayer(4.5);
    expect(await screen.findByRole('button', { name: '重试读取' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '添加时间点（M）' })).toBeDisabled();
    fireEvent.keyDown(window, { key: 'm' });
    expect(api.saveMarks).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: '重试读取' }));
    await loaded();
    vi.mocked(api.saveMarks).mockRejectedValueOnce(new Error('disk full'));
    fireEvent.keyDown(window, { key: 'm' });
    const retry = await screen.findByRole('button', { name: '重试保存' });
    expect(screen.getByText('保存失败，仍有未保存更改')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '编辑 标记点' })).toBeInTheDocument();
    fireEvent.click(retry);
    await saved();
    expect(screen.getByText('所有更改已保存到本地')).toBeInTheDocument();
    expect(api.saveMarks).toHaveBeenCalledTimes(2);
  });

  it('isolates late selection loads, discards pending intervals, and completes old saves after unmount', async () => {
    const late = deferred<MarksDocument>();
    vi.mocked(api.getMarks).mockImplementationOnce(() => late.promise);
    const old = video;
    const second = { ...video, id: `${video.id}-second`, name: '第二个视频.mp4', path: 'C:/second.mp4' };
    useAppStore.setState({ videos: [old, second] });
    const view = render(<MarksPage />);
    readyPlayer(3);
    fireEvent.change(screen.getByLabelText('当前视频'), { target: { value: second.id } });
    video = second;
    await loaded();
    const player = readyPlayer(20.75);
    fireEvent.keyDown(window, { key: 'r' });
    await act(async () => { late.resolve(document(old.id, [pluginMark])); });
    expect(screen.queryByRole('button', { name: '编辑 插件标注' })).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('当前视频'), { target: { value: old.id } });
    video = old;
    await loaded();
    readyPlayer(5);
    expect(screen.getByRole('button', { name: '开始区间（R）' })).toBeInTheDocument();
    const pendingSave = deferred<MarksDocument>();
    vi.mocked(api.saveMarks).mockImplementationOnce(() => pendingSave.promise);
    fireEvent.keyDown(window, { key: 'm' });
    const snapshot = marksEntry(old.id).marks;
    view.unmount();
    await act(async () => { pendingSave.resolve(document(old.id, snapshot)); });
    await saved();
    expect(marksEntry(second.id).marks).toEqual([]);
    expect(player.isConnected).toBe(false);
    render(<MarksPage />);
    expect(screen.getByRole('button', { name: '编辑 标记点' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '编辑 插件标注' })).toBeInTheDocument();
  });

  it('disables zero-duration and failed playback controls, and can retry playback', async () => {
    render(<MarksPage />);
    await loaded();
    const player = readyPlayer(0, 0);
    expect(screen.getByRole('slider', { name: '播放位置' })).toBeDisabled();
    fireEvent.keyDown(window, { key: 'm' });
    expect(api.saveMarks).not.toHaveBeenCalled();
    Object.defineProperty(player, 'duration', { configurable: true, value: 100 });
    fireEvent.durationChange(player);
    expect(screen.getByRole('button', { name: '添加时间点（M）' })).not.toBeDisabled();
    fireEvent.error(player);
    expect(screen.getByRole('button', { name: '添加时间点（M）' })).toBeDisabled();
    const reload = vi.spyOn(player, 'load').mockImplementation(() => {});
    fireEvent.click(screen.getByRole('button', { name: '重新加载视频' }));
    expect(reload).toHaveBeenCalledTimes(1);
    fireEvent.loadedMetadata(player);
    expect(screen.getByRole('button', { name: '添加时间点（M）' })).not.toBeDisabled();
  });

  it('shows visible labels and prevents blank rename writes', async () => {
    vi.mocked(api.getMarks).mockResolvedValueOnce(document(video.id, [pluginMark]));
    render(<MarksPage />);
    await loaded();
    fireEvent.click(screen.getByRole('button', { name: '编辑 插件标注' }));
    const input = screen.getByRole('textbox', { name: '标注名称' });
    expect(screen.getByText('标注名称', { selector: 'label' })).toBeVisible();
    fireEvent.change(input, { target: { value: '  ' } });
    expect(screen.getByRole('button', { name: '保存名称' })).toBeDisabled();
    fireEvent.submit(input.closest('form')!);
    expect(api.saveMarks).not.toHaveBeenCalled();
    expect(within(screen.getByRole('list', { name: '标注列表' })).getByText('名称不能为空')).toBeInTheDocument();
  });
});
