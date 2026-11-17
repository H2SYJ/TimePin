import type { Mark } from '@/types';

/**
 * Only finite, non-negative, strictly-positive-length intervals inside the known
 * duration can be exported as segments. Time points (endTime === null), non-finite
 * bounds, zero/negative length and out-of-range intervals are never selectable.
 */
export function isExportableInterval(mark: Mark, duration: number): boolean {
  return (
    mark.endTime !== null
    && Number.isFinite(mark.startTime)
    && Number.isFinite(mark.endTime)
    && mark.startTime >= 0
    && mark.endTime > mark.startTime
    && Number.isFinite(duration)
    && duration > 0
    && mark.endTime <= duration
  );
}

/** Marks eligible for export; identity order preserved. */
export function exportableMarks(marks: Mark[], duration: number): Mark[] {
  return marks.filter((mark) => isExportableInterval(mark, duration));
}

/** Selected ids that are still valid (kept order, no duplicates). */
export function validSelectedIds(selected: Iterable<string>, marks: Mark[], duration: number): string[] {
  const allowed = new Set(exportableMarks(marks, duration).map((mark) => mark.id));
  const seen = new Set<string>();
  const result: string[] = [];
  for (const id of selected) {
    if (allowed.has(id) && !seen.has(id)) {
      seen.add(id);
      result.push(id);
    }
  }
  return result;
}
