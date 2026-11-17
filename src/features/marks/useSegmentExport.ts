import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { api, isTauri } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import { useUpdateStore } from '@/stores/updater';
import type { Mark, VideoInfo } from '@/types';
import { exportableMarks, validSelectedIds } from './exportMarks';

export type ExportPhase = 'idle' | 'picking' | 'exporting';

export interface ExportState {
  phase: ExportPhase;
  /** Truthful result message; success lists actual returned paths. */
  message: string | null;
  /** Readable error; selection is retained so the user can retry. */
  error: string | null;
  /** Output files reported by the last successful export. */
  outputPaths: string[];
}

/**
 * Interval selection + segment export flow.
 *
 * Selection lives only in React state — it never seeks, never persists, and is
 * revalidated against live marks/duration every render. Deleting or editing a
 * mark silently drops it from the selection.
 *
 * Duplicate submissions are blocked while the directory picker is pending and
 * while an export request is in flight. The request payload is snapshotted
 * before any await. A dialog or IPC resolving after unmount / video switch is
 * ignored for UI updates, but the updater work token is still released.
 */
export function useSegmentExport(video: VideoInfo, marks: Mark[], duration: number) {
  const [rawSelectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [state, setState] = useState<ExportState>({ phase: 'idle', message: null, error: null, outputPaths: [] });
  // Flip synchronously at click time: a fast double-click must not reach the
  // picker or IPC twice because a setState render has not committed yet.
  const busy = useRef(false);
  const generation = useRef(0);

  const exportable = useMemo(() => exportableMarks(marks, duration), [marks, duration]);
  const exportableIds = useMemo(() => new Set(exportable.map((mark) => mark.id)), [exportable]);

  // Revalidate the selection against the current marks and known duration:
  // deleted marks, point marks and out-of-range intervals drop out on their own.
  const selectedIds = useMemo(() => {
    const kept = validSelectedIds(rawSelectedIds, marks, duration);
    return kept.length === rawSelectedIds.size ? rawSelectedIds : new Set(kept);
  }, [rawSelectedIds, marks, duration]);
  const selectedMarks = useMemo(() => exportable.filter((mark) => selectedIds.has(mark.id)), [exportable, selectedIds]);

  const toggleSelected = useCallback((markId: string, selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(markId);
      else next.delete(markId);
      return next;
    });
  }, []);

  const toggleAll = useCallback((selected: boolean, allIds: string[]) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      for (const id of allIds) {
        if (selected) next.add(id);
        else next.delete(id);
      }
      return next;
    });
  }, []);

  const runExport = useCallback(() => {
    if (busy.current) return;
    // Snapshot before the first await so later edits/selection changes cannot
    // mutate a request that is already being assembled.
    const snapshot = { videoPath: video.path, marks: marks.filter((mark) => selectedIds.has(mark.id)) };
    const exportMode = useAppStore.getState().settings.defaultExportMode ?? 'streamCopy';

    if (!isTauri()) {
      setState({ phase: 'idle', message: null, error: '片段导出需要在 TimePin 桌面应用中运行', outputPaths: [] });
      return;
    }
    if (exportMode === 'frameAccurate') {
      setState({
        phase: 'idle',
        message: null,
        error: '默认导出模式为「帧精确」，但后端尚未支持该模式。请在设置中改回「流复制」后重试；不会自动降级或重编码。',
        outputPaths: [],
      });
      return;
    }
    if (snapshot.marks.length === 0) {
      setState({ phase: 'idle', message: null, error: '请先勾选至少一个可导出的区间', outputPaths: [] });
      return;
    }

    busy.current = true;
    const gen = ++generation.current;
    setState({ phase: 'picking', message: null, error: null, outputPaths: [] });
    void (async () => {
      const finish = (next: ExportState) => {
        busy.current = false;
        if (generation.current === gen) setState(next);
      };
      let dir: string | null;
      try {
        dir = await api.pickDirectory('选择导出目录');
      } catch (error) {
        finish({ phase: 'idle', message: null, error: `无法打开目录选择：${String(error)}`, outputPaths: [] });
        return;
      }
      if (generation.current !== gen) { busy.current = false; return; } // superseded / unmounted
      if (dir === null) {
        // Directory cancel is harmless: keep selection, no error.
        finish({ phase: 'idle', message: null, error: null, outputPaths: [] });
        return;
      }
      setState({ phase: 'exporting', message: null, error: null, outputPaths: [] });
      const token = useUpdateStore.getState().beginWork('export');
      try {
        const paths = await api.exportSegments(snapshot.videoPath, snapshot.marks, dir, exportMode);
        finish({
          phase: 'idle',
          message: paths.length > 0 ? '导出完成' : '导出完成，但后端未返回输出文件路径',
          error: null,
          outputPaths: paths,
        });
      } catch (error) {
        finish({ phase: 'idle', message: null, error: `导出失败：${String(error)}`, outputPaths: [] });
      } finally {
        // Release the updater lock even if this component unmounted mid-export.
        useUpdateStore.getState().endWork(token);
      }
    })();
  }, [marks, video.path, selectedIds]);

  // Invalidate any in-flight continuation on unmount / video switch.
  useEffect(() => {
    return () => { generation.current += 1; };
  }, [video.id, video.path]);

  return {
    selectedIds,
    selectedMarks,
    exportableIds,
    exportable,
    toggleSelected,
    toggleAll,
    state,
    runExport,
  };
}
