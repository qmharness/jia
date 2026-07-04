import { t } from './i18n';

export function relativeTime(ts: number): string {
  const now = Date.now();
  const diff = now - ts * 1000;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return t('time.justNow');
  if (mins < 60) return t('time.mAgo', { n: mins });
  const hours = Math.floor(mins / 60);
  if (hours < 24) return t('time.hAgo', { n: hours });
  const days = Math.floor(hours / 24);
  if (days < 7) return t('time.dAgo', { n: days });
  return new Date(ts * 1000).toLocaleDateString();
}

export type TimeGroup = 'Today' | 'Yesterday' | 'This week' | 'This month' | 'Older';

export function formatTime(ts: number): string {
  const d = new Date(ts * 1000);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

export function groupKey(ts: number): TimeGroup {
  const now = new Date();
  const d = new Date(ts * 1000);
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startOfYesterday = new Date(startOfToday.getTime() - 86400000);
  const startOfWeek = new Date(startOfToday.getTime() - startOfToday.getDay() * 86400000);
  const startOfMonth = new Date(now.getFullYear(), now.getMonth(), 1);

  if (d >= startOfToday) return 'Today';
  if (d >= startOfYesterday) return 'Yesterday';
  if (d >= startOfWeek) return 'This week';
  if (d >= startOfMonth) return 'This month';
  return 'Older';
}
