// TypeScript mirror of the `/api/v1` contract (ARCHITECTURE §6, docs/api/examples/*.json).
// Hand-maintained: `fixtures.ts` type-checks every example file against these types, so drift
// fails `pnpm check`. Optional values are always present as `null`, never omitted.

// Closed string sets are declared as runtime arrays so tests can assert example values are members.
export const HEALTH_STATES = ['healthy', 'warning', 'critical', 'unknown'] as const;
export type Health = (typeof HEALTH_STATES)[number];

export const TARGET_KINDS = ['system', 'service', 'timer', 'check'] as const;
export type TargetKind = (typeof TARGET_KINDS)[number];

export const CHECK_KINDS = ['http', 'tcp'] as const;
export type CheckKind = (typeof CHECK_KINDS)[number];

export const RUN_SOURCES = ['observed', 'journal'] as const;
export type RunSource = (typeof RUN_SOURCES)[number];

export const RESOLUTIONS = ['raw', '5m'] as const;
export type Resolution = (typeof RESOLUTIONS)[number];

// ARCHITECTURE §5, series keys table.
// `count` (service.restarts:<unit>) is announced by the backend track for M3.5.
export const SERIES_UNITS = [
  'percent',
  'load',
  'bytes',
  'bytes_per_s',
  'celsius',
  'ms',
  'count',
] as const;
export type SeriesUnit = (typeof SERIES_UNITS)[number];

/** Unix seconds (integer). */
export type UnixSeconds = number;

// ---- GET /api/v1/health ----------------------------------------------------------------------

export interface HealthResponse {
  status: 'ok';
  version: string;
}

// ---- Errors (any 4xx/5xx) ----------------------------------------------------------------------

export interface ApiError {
  error: {
    code: string;
    message: string;
  };
}

// ---- Shared items --------------------------------------------------------------------------------

/** A health transition of one target (named to avoid shadowing the DOM `Event`). */
export interface HealthEvent {
  id: number;
  ts: UnixSeconds;
  targetKind: TargetKind;
  target: string;
  /** Null for the first observed state of a target (no previous state). */
  from: Health | null;
  to: Health;
  message: string | null;
}

export interface ServiceSummary {
  unit: string;
  name: string;
  description: string;
  health: Health;
  message: string | null;
  loadState: string;
  activeState: string;
  subState: string;
  since: UnixSeconds | null;
  restarts: number;
  /** Bytes. */
  memory: number | null;
}

/** `lastRun` of a timer. `finishedAt` etc. are null while the run is still in progress. */
export interface TimerRunSummary {
  startedAt: UnixSeconds;
  finishedAt: UnixSeconds | null;
  /** Seconds. */
  duration: number | null;
  /** systemd `Result=`, e.g. `success`, `exit-code`, `timeout`. */
  result: string | null;
  exitStatus: number | null;
}

export interface TimerRun extends TimerRunSummary {
  source: RunSource;
}

export interface TimerSummary {
  unit: string;
  name: string;
  service: string;
  health: Health;
  message: string | null;
  active: boolean;
  /** Null if the timer has never run. */
  lastRun: TimerRunSummary | null;
  nextRun: UnixSeconds | null;
  running: boolean;
}

export interface CheckSummary {
  name: string;
  kind: CheckKind;
  /** Configured URL or address with userinfo and query string stripped. */
  target: string;
  health: Health;
  message: string | null;
  /** Null until the check has run once. */
  lastCheck: UnixSeconds | null;
  latencyMs: number | null;
  consecutiveFailures: number;
}

// ---- GET /api/v1/overview ------------------------------------------------------------------------

export interface HostInfo {
  hostname: string;
  os: string;
  kernel: string;
  cpuModel: string;
  cores: number;
  bootTime: UnixSeconds;
  /** Seconds. */
  uptime: number;
}

export interface CpuMetrics {
  /** Percent, instantaneous. */
  usage: number;
  /** Percent. */
  iowait: number;
  health: Health;
}

export interface LoadMetrics {
  one: number;
  five: number;
  fifteen: number;
  health: Health;
}

/** Sizes in bytes; `used` is `total - available`. */
export interface MemoryMetrics {
  total: number;
  used: number;
  usedPct: number;
  swapTotal: number;
  swapUsed: number;
  health: Health;
}

export interface DiskMetrics {
  mount: string;
  fsType: string;
  total: number;
  used: number;
  usedPct: number;
  health: Health;
}

export interface NetMetrics {
  iface: string;
  rxBps: number;
  txBps: number;
}

export interface TempMetrics {
  label: string;
  celsius: number;
  health: Health;
}

export interface SystemMetrics {
  health: Health;
  ts: UnixSeconds;
  cpu: CpuMetrics;
  load: LoadMetrics;
  memory: MemoryMetrics;
  disks: DiskMetrics[];
  /** Empty means "not available". */
  net: NetMetrics[];
  /** Empty means "not available". */
  temps: TempMetrics[];
}

export type HealthCounts = Record<Health, number>;

export interface Overview {
  host: string;
  ts: UnixSeconds;
  health: Health;
  hostInfo: HostInfo;
  system: SystemMetrics;
  counts: HealthCounts;
  services: ServiceSummary[];
  timers: TimerSummary[];
  checks: CheckSummary[];
  /** Newest first, at most 10. */
  recentEvents: HealthEvent[];
}

// ---- List and detail endpoints -------------------------------------------------------------------

export interface ServicesResponse {
  host: string;
  services: ServiceSummary[];
}

export interface ServiceDetail {
  host: string;
  service: ServiceSummary;
  events: HealthEvent[];
}

export interface TimersResponse {
  host: string;
  timers: TimerSummary[];
}

export interface TimerDetail {
  host: string;
  timer: TimerSummary;
  /** Newest first. */
  runs: TimerRun[];
  events: HealthEvent[];
}

export interface ChecksResponse {
  host: string;
  checks: CheckSummary[];
}

export interface CheckDetail {
  host: string;
  check: CheckSummary;
  events: HealthEvent[];
}

export interface EventsResponse {
  host: string;
  /** Newest first. */
  events: HealthEvent[];
}

// ---- Metrics -------------------------------------------------------------------------------------

export interface SeriesInfo {
  /** `<metric>[:<label>]`, e.g. `disk.used:/`. */
  key: string;
  metric: string;
  label: string | null;
  unit: SeriesUnit;
}

export interface MetricsResponse {
  host: string;
  series: SeriesInfo[];
}

/** `[ts, avg, min, max]`; empty buckets are null so charts show gaps. */
export type MetricPoint = [
  ts: UnixSeconds,
  avg: number | null,
  min: number | null,
  max: number | null,
];

export interface QuerySeries {
  key: string;
  unit: SeriesUnit;
  points: MetricPoint[];
}

export interface MetricsQuery {
  host: string;
  from: UnixSeconds;
  to: UnixSeconds;
  /** Bucket width in seconds. */
  step: number;
  resolution: Resolution;
  series: QuerySeries[];
}
