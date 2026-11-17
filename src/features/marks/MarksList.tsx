import { useState } from 'react';
import { ChevronRight, Pencil, Trash2 } from 'lucide-react';
import { Button, Checkbox, Input } from '@/components/ui';
import type { Mark } from '@/types';
import { isExportableInterval } from './exportMarks';
import { formatTime } from './format';

interface MarksListProps {
  marks: Mark[];
  readOnly: boolean;
  canSeek: boolean;
  duration: number;
  /** Which exportable marks are currently selected for segment export. */
  selectedIds?: Set<string>;
  onToggleExport?: (markId: string, selected: boolean) => void;
  exportBusy?: boolean;
  onSeek: (time: number) => void;
  onRename: (mark: Mark, label: string) => void;
  onDelete: (mark: Mark) => void;
}

function MarkRow({ mark, readOnly, canSeek, duration, selectedIds, onToggleExport, exportBusy, onSeek, onRename, onDelete }: Omit<MarksListProps, 'marks'> & { mark: Mark }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(mark.label);
  const exportable = isExportableInterval(mark, duration);
  const commit = () => {
    if (!draft.trim() || readOnly) return;
    onRename(mark, draft);
    setEditing(false);
  };

  return (
    <li className="rounded-xl border border-divider bg-background/60 p-3">
      <div className="flex items-center gap-2">
        {onToggleExport ? (
          <Checkbox
            checked={selectedIds?.has(mark.id) ?? false}
            onCheckedChange={(checked) => onToggleExport(mark.id, checked)}
            isDisabled={readOnly || !exportable || exportBusy}
            aria-label={`导出 ${mark.label}`}
            className="shrink-0"
          />
        ) : null}
        <Button
          variant="ghost" className="min-w-0 flex-1 justify-start px-2 text-left"
          aria-label={`跳转到 ${mark.label}`} isDisabled={!canSeek}
          onPress={() => onSeek(mark.startTime)}
        >
          <ChevronRight className="h-4 w-4 shrink-0" aria-hidden="true" />
          <span className="truncate">{mark.label}</span>
        </Button>
        <Button size="icon" variant="ghost" aria-label={`编辑 ${mark.label}`} isDisabled={readOnly}
          onPress={() => { setDraft(mark.label); setEditing(true); }}>
          <Pencil className="h-4 w-4" aria-hidden="true" />
        </Button>
        <Button size="icon" variant="danger" aria-label={`删除 ${mark.label}`} isDisabled={readOnly}
          onPress={() => onDelete(mark)}>
          <Trash2 className="h-4 w-4" aria-hidden="true" />
        </Button>
      </div>
      <p className="mt-1 font-mono text-xs tabular-nums text-default-500">
        {mark.endTime === null ? '点' : '区间'} · {formatTime(mark.startTime)}
        {mark.endTime !== null ? ` – ${formatTime(mark.endTime)}` : ''}
        {onToggleExport && !exportable ? (
          <span className="ml-2 text-default-400">
            {mark.endTime === null ? '· 不可导出（时间点）' : '· 不可导出（无效或超出时长）'}
          </span>
        ) : null}
      </p>
      <p className="mt-1 text-xs text-default-500">
        来源：{mark.sourcePlugin ?? '手动'}
        {mark.confidence !== null ? ` · 置信度 ${(mark.confidence * 100).toFixed(1)}%` : ''}
        {mark.thumbnailPath ? ' · 含截图证据' : ''}
      </p>
      {editing ? (
        <form className="mt-3 space-y-2" onSubmit={(event) => { event.preventDefault(); commit(); }}>
          <label id={`mark-name-label-${mark.id}`} htmlFor={`mark-name-${mark.id}`} className="block text-sm font-medium">标注名称</label>
          <Input id={`mark-name-${mark.id}`} aria-labelledby={`mark-name-label-${mark.id}`} value={draft} onValueChange={setDraft}
            isDisabled={readOnly} autoFocus
            onKeyDown={(event) => { if (event.key === 'Escape') setEditing(false); }} />
          {!draft.trim() ? <p className="text-xs text-danger">名称不能为空</p> : null}
          <div className="flex gap-2">
            <Button size="sm" type="submit" isDisabled={readOnly || !draft.trim()}>保存名称</Button>
            <Button size="sm" variant="secondary" onPress={() => setEditing(false)}>取消编辑</Button>
          </div>
        </form>
      ) : null}
    </li>
  );
}

export function MarksList({ marks, ...props }: MarksListProps) {
  if (marks.length === 0) {
    return <p className="rounded-xl border border-dashed border-divider p-6 text-sm leading-relaxed text-default-500">暂无标注。播放视频时按 M 打点，按 R 开始/结束区间。</p>;
  }
  return (
    <ul aria-label="标注列表" className="flex flex-col gap-3">
      {marks.map((mark) => <MarkRow key={mark.id} mark={mark} {...props} />)}
    </ul>
  );
}
