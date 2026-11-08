// Timestamps on chart axes, tooltips and tables, in local time like uPlot's axis. Times are
// 24-hour (`09:00`, 5 characters in every locale) so tick labels stay compact.

const DAY = 86_400;

const TIME: Intl.DateTimeFormatOptions = { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' };

/**
 * A bucket time for tooltips and tables. The bare time only when it is on the same local day as
 * `reference` (the newest bucket), so "07:30" on a 24h chart can never mean yesterday.
 */
export function formatChartTime(ts: number, reference: number): string {
  const date = new Date(ts * 1000);
  if (sameLocalDay(date, new Date(reference * 1000)))
    return date.toLocaleTimeString(undefined, TIME);
  return date.toLocaleString(undefined, { month: 'short', day: 'numeric', ...TIME });
}

function sameLocalDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

const HOUR = 3_600;

/**
 * Time-axis tick labels. `increment` is the tick spacing in seconds: below a day ticks show the
 * time of day, with the date at midnight so a 24h axis says where the day turns; from a day up
 * they show dates, and from about a month the month name alone.
 */
export function formatTimeTicks(ticks: readonly number[], increment: number): string[] {
  return ticks.map((ts) => {
    const date = new Date(ts * 1000);
    if (increment >= 28 * DAY) {
      return date.toLocaleDateString(undefined, { month: 'short', year: 'numeric' });
    }
    const midnight = date.getHours() === 0 && date.getMinutes() === 0;
    if (increment >= DAY || (increment >= HOUR && midnight)) {
      return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    }
    return date.toLocaleTimeString(undefined, TIME);
  });
}
