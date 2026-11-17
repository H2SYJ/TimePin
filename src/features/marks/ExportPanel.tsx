import { useMemo } from 'react';
import { Download, Loader2 } from 'lucide-react';
import { Button, Card, Checkbox } from '@/components/ui';
import { useAppStore } from '@/stores/app';
import type { ExportMode, Mark } from '@/types';
import { exportableMarks } from './exportMarks';
import type { ExportState } from './useSegmentExport';
import { formatTime } from './format';

interface ExportPanelProps {
  marks: Mark[];
  duration: number;
  selectedIds: Set<string>;
  onToggle: (markId: string, selected: boolean) => void;
  onToggleAll: (selected: boolean, allIds: string[]) => void;
  state: ExportState;
  onExport: () => void;
  canInteract: boolean;
}

/**
 * Segment export controls for the marks workspace. Only valid intervals are
 * selectable; time points are shown as non-exportable. Export uses FFmpeg
 * stream copy — the pre-export caveat about overwrite (-y) and partial output
 * is always visible so the user knows what the backend actually does.
 */
const EXPORT_MODE_LABEL: Record<ExportMode, string> = {
  streamCopy: '流复制（-c copy）',
  frameAccurate: '帧精确（尚未支持）',
};

export function ExportPanel({ marks, duration, selectedIds, onToggle, onToggleAll, state, onExport, canInteract }: ExportPanelProps) {
  const exportable = useMemo(() => exportableMarks(marks, duration), [marks, duration]);
  const exportableIds = useMemo(() => exportable.map((mark) => mark.id), [exportable]);
  const allSelected = exportableIds.length > 0 && exportableIds.every((id) => selectedIds.has(id));
  const noneSelected = selectedIds.size === 0;
  const busy = state.phase !== 'idle';
  const exportMode = useAppStore((state) => state.settings.defaultExportMode);

  const statusLabel = state.phase === 'picking' ? '等待选择导出目录…' : state.phase === 'exporting' ? '正在导出片段…' : null;

  return (
    <Card className="rounded-2xl border-divider bg-default-50/50 p-4" aria-label="片段导出">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="min-w-0">
          <h3 className="text-sm font-semibold">片段导出</h3>
          <p className="mt-0.5 text-xs text-default-500">
            已选 {selectedIds.size}/{exportable.length} 个区间 · {EXPORT_MODE_LABEL[exportMode]}，不重编码、不修改原视频
          </p>
        </div>
        <Button
          isDisabled={busy || noneSelected || exportable.length === 0 || !canInteract}
          onPress={onExport}
          variant="primary"
          aria-label="导出所选区间"
        >
          {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" /> : <Download className="h-4 w-4" aria-hidden="true" />}
          {statusLabel ?? '导出所选区间'}
        </Button>
      </div>

      <p className="mt-3 text-xs leading-relaxed text-default-500">
        导出前请确认：输出目录中同名文件会被覆盖（-y）；导出按区间串行执行，中途失败可能留下部分已生成的文件，不保证原子性。
      </p>

      {exportable.length > 0 ? (
        <div className="mt-3 space-y-2">
          <Checkbox
            checked={allSelected}
            onCheckedChange={(checked) => onToggleAll(checked, exportableIds)}
            isDisabled={!canInteract || busy}
            label="全选所有可导出区间"
          />
          <ul aria-label="可导出区间" className="space-y-1">
            {exportable.map((mark) => (
              <li key={mark.id}>
                <Checkbox
                  checked={selectedIds.has(mark.id)}
                  onCheckedChange={(checked) => onToggle(mark.id, checked)}
                  isDisabled={!canInteract || busy}
                  aria-label={`选择区间 ${mark.label}`}
                  label={`${mark.label} · ${formatTime(mark.startTime)} – ${formatTime(mark.endTime ?? mark.startTime)}`}
                />
              </li>
            ))}
          </ul>
        </div>
      ) : duration <= 0 && marks.length > 0 ? (
        <p className="mt-3 text-xs text-default-500">
          正在读取视频时长，播放器就绪后可选择导出区间…
        </p>
      ) : (
        <p className="mt-3 text-xs text-default-500">
          没有可导出的区间。只有完整区间（结束时间晚于开始时间且在视频时长内）可导出；时间点不可作为片段导出。
        </p>
      )}

      {state.message ? (
        <div role="status" aria-live="polite" className="mt-3 rounded-xl border border-success/30 bg-success/10 p-3">
          <p className="text-sm text-success">{state.message}</p>
          {state.outputPaths.length > 0 ? (
            <ul className="mt-2 space-y-1 font-mono text-xs text-default-600">
              {state.outputPaths.map((path) => <li key={path} className="break-all">· {path}</li>)}
            </ul>
          ) : null}
        </div>
      ) : null}
      {state.error ? (
        <div role="alert" className="mt-3 rounded-xl border border-danger/30 bg-danger/10 p-3">
          <p className="text-sm text-danger">{state.error}</p>
        </div>
      ) : null}
      {statusLabel ? (
        <p role="status" aria-live="polite" className="mt-2 text-xs text-default-500">{statusLabel}</p>
      ) : null}
    </Card>
  );
}
