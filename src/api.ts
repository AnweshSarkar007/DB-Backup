// Typed wrappers around the Tauri command layer (mirrors the Rust DTOs in src-tauri/src/commands).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export interface DockerHealth {
  installed: boolean;
  running: boolean;
  version: string | null;
}

export interface ConnInfo {
  database: string;
  host: string;
  username: string;
  postgres_major: number;
  postgres_text: string;
  size_bytes: number | null;
  engine_image: string;
}

export interface ConnectionConfig {
  name: string;
  host: string;
  port: number;
  database: string;
  username: string;
  ssl_mode: string;
  postgres_major_version?: number | null;
}

export interface ConnectionRecord {
  id: number;
  uuid: string;
  config: ConnectionConfig;
  secret_reference: string;
  created_at: string;
  updated_at: string;
}

export interface DestinationRecord {
  id: number;
  uuid: string;
  name: string;
  kind: string;
  configuration_json: string;
  secret_reference: string | null;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface ErrorReport {
  code: string;
  message: string;
  detail: string;
  retryable: boolean;
}

export type DestinationStatus = "completed" | "failed" | "skipped";

export interface DestinationResult {
  name: string;
  status: DestinationStatus;
  stored_path?: string | null;
  bytes_written?: number | null;
  error_code?: string | null;
  error_message?: string | null;
}

export interface RunResult {
  backup_id: string;
  job_id: string;
  state: string;
  filename: string | null;
  local_temp_path: string | null;
  file_size: number | null;
  sha256: string | null;
  postgres_major_version: number | null;
  verification: string;
  destinations: DestinationResult[];
  started_at: string | null;
  completed_at: string | null;
  error_code: string | null;
  error_message: string | null;
}

export interface RunSummary {
  id: number;
  uuid: string;
  connection_id: number | null;
  policy_id: number | null;
  trigger_type: string;
  database_label: string | null;
  started_at: string | null;
  completed_at: string | null;
  status: string;
  filename: string | null;
  file_size: number | null;
  sha256: string | null;
  verification_status: string | null;
  error_code: string | null;
  error_message: string | null;
}

export interface ProgressEvent {
  state: string;
  message: string;
}

export interface Schedule {
  frequency: "daily" | "weekly";
  time: string; // HH:MM
  day_of_week?: string | null; // SUN..SAT
}

export interface RetentionPolicy {
  keep_last?: number | null;
  keep_days?: number | null;
}

export interface PolicyRecord {
  id: number;
  uuid: string;
  connection_id: number;
  name: string;
  schedule_json: string | null;
  retention_policy_json: string | null;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export interface TaskStatus {
  exists: boolean;
  next_run: string | null;
  last_run: string | null;
  last_result: string | null;
}

export interface ScheduleInfo {
  policy: PolicyRecord;
  connection_name: string;
  schedule: Schedule | null;
  destination_ids: number[];
  task: TaskStatus;
}

// --- Docker ---
export const dockerHealth = (): Promise<DockerHealth> => invoke("docker_health");

// --- Connections ---
export const addConnection = (name: string, uri: string): Promise<ConnectionRecord> =>
  invoke("add_connection", { name, uri });
export const listConnections = (): Promise<ConnectionRecord[]> => invoke("list_connections");
export const deleteConnection = (id: number): Promise<void> => invoke("delete_connection", { id });
export const testConnection = (id: number): Promise<ConnInfo> => invoke("test_connection", { id });
export const testConnectionUri = (uri: string, name: string): Promise<ConnInfo> =>
  invoke("test_connection_uri", { uri, name });

// --- Destinations ---
export const addLocalDestination = (name: string, path: string): Promise<DestinationRecord> =>
  invoke("add_local_destination", { name, path });
export interface GoogleOAuthImportResult { client_id: string; accounts_reconnect_required: number; same_client_id: boolean }
export const importGoogleOAuthJson = (path: string): Promise<GoogleOAuthImportResult> =>
  invoke("import_google_oauth_json", { path });
export const setGoogleClient = (clientId: string, clientSecret: string): Promise<GoogleOAuthImportResult> =>
  invoke("set_google_client", { clientId, clientSecret });
export const googleClientConfigured = (): Promise<boolean> => invoke("google_client_configured");
export const connectGoogleDrive = (name: string): Promise<DestinationRecord> =>
  invoke("connect_google_drive", { name });
export const reconnectGoogleDrive = (destinationId: number): Promise<DestinationRecord> =>
  invoke("reconnect_google_drive", { destinationId });
export const listDestinations = (): Promise<DestinationRecord[]> => invoke("list_destinations");
export const setDestinationEnabled = (id: number, enabled: boolean): Promise<void> =>
  invoke("set_destination_enabled", { id, enabled });
export const deleteDestination = (id: number): Promise<void> => invoke("delete_destination", { id });

// --- Backup + history ---
export const backupNow = (connectionId: number, destinationIds: number[]): Promise<RunResult> =>
  invoke("backup_now", { connectionId, destinationIds });
export const runLocalBackup = (
  uri: string,
  name: string,
  destName: string,
  destPath: string
): Promise<RunResult> => invoke("run_local_backup", { uri, name, destName, destPath });
export const listRuns = (
  connectionId: number | null,
  status: string | null,
  limit?: number
): Promise<RunSummary[]> => invoke("list_runs", { connectionId, status, limit: limit ?? 200 });
export const getRun = (uuid: string): Promise<RunSummary> => invoke("get_run", { uuid });

// --- Schedules ---
export const createSchedule = (
  connectionId: number,
  name: string,
  destinationIds: number[],
  schedule: Schedule,
  retention: RetentionPolicy | null
): Promise<PolicyRecord> =>
  invoke("create_schedule", { connectionId, name, destinationIds, schedule, retention });
export const listSchedules = (): Promise<ScheduleInfo[]> => invoke("list_schedules");
export const setScheduleEnabled = (id: number, enabled: boolean): Promise<void> =>
  invoke("set_schedule_enabled", { id, enabled });
export const deleteSchedule = (id: number): Promise<void> => invoke("delete_schedule", { id });
export const runScheduleNow = (id: number): Promise<RunResult> =>
  invoke("run_schedule_now", { id });

// --- Inspect / Browse / Restore ---
export interface ArchiveEntry {
  object_type: string;
  schema: string;
  name: string;
}
export interface TableInfo {
  schema: string;
  name: string;
  estimated_rows: number;
}
export interface BrowseOpenResult {
  tables: TableInfo[];
  restore_warning: string | null;
}
export interface TablePage {
  columns: string[];
  rows: (string | null)[][];
  offset: number;
  limit: number;
  restore_warning: string | null;
}

export const inspectDump = (path: string): Promise<ArchiveEntry[]> =>
  invoke("inspect_dump", { path });
export const browseOpen = (path: string): Promise<BrowseOpenResult> => invoke("browse_open", { path });
export const browseTable = (
  schema: string,
  table: string,
  offset: number,
  limit: number
): Promise<TablePage> => invoke("browse_table", { schema, table, offset, limit });
export const browseClose = (): Promise<void> => invoke("browse_close");
export const restoreBackup = (
  dumpPath: string,
  targetConnectionId: number,
  confirmText: string
): Promise<void> => invoke("restore_backup", { dumpPath, targetConnectionId, confirmText });

export const onBackupProgress = (cb: (e: ProgressEvent) => void): Promise<UnlistenFn> =>
  listen<ProgressEvent>("backup-progress", (evt) => cb(evt.payload));

export function humanBytes(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let size = bytes;
  let u = 0;
  while (size >= 1024 && u < units.length - 1) {
    size /= 1024;
    u += 1;
  }
  return u === 0 ? `${bytes} B` : `${size.toFixed(1)} ${units[u]}`;
}

export function fmtDate(s: string | null | undefined): string {
  if (!s) return "—";
  const d = new Date(s);
  return isNaN(d.getTime()) ? s : d.toLocaleString();
}
