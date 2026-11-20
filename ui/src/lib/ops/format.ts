import type { FileFacts } from '$lib/ipc/bindings/FileFacts';
import type { JobView } from '$lib/ipc/bindings/JobView';
import { formatDate, formatSize } from '$lib/panes/format';

/** `12 MB/s` */
export function formatSpeed(bytesPerSec: number): string {
  return `${formatSize(bytesPerSec)}/s`;
}

/** `42 s`, `3 min 5 s`, `2 h 5 min`: the two biggest units, seconds only below a minute. */
export function formatEta(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const secs = total % 60;

  if (hours > 0) {
    return minutes > 0 ? `${hours} h ${minutes} min` : `${hours} h`;
  }

  if (minutes > 0) {
    return secs > 0 ? `${minutes} min ${secs} s` : `${minutes} min`;
  }

  return `${secs} s`;
}

/** `12 MB/s · 3 min left`; a part that is not known yet is left out. */
export function formatRate(job: Pick<JobView, 'bytesPerSec' | 'etaSecs'>): string {
  const parts: string[] = [];
  if (job.bytesPerSec > 0) {
    parts.push(formatSpeed(job.bytesPerSec));
  }

  if (job.etaSecs !== null) {
    parts.push(`${formatEta(job.etaSecs)} left`);
  }

  return parts.join(' · ');
}

/** Whole percent of the bytes done, or of the files when there are no bytes (empty files); 0 before the total is known. */
export function percentDone(job: Pick<JobView, 'doneBytes' | 'totalBytes' | 'doneFiles' | 'totalFiles'>): number {
  const [done, total] = job.totalBytes > 0 ? [job.doneBytes, job.totalBytes] : [job.doneFiles, job.totalFiles];

  return total > 0 ? Math.min(100, Math.floor((done / total) * 100)) : 0;
}

/** `4.2 MB · 2026-11-20 07:58`; a folder has no size to show. */
export function formatFacts(facts: FileFacts): string {
  const size = facts.dir ? 'Folder' : formatSize(facts.size);

  return [size, formatDate(facts.modified)].filter((part) => part !== '').join(' · ');
}
