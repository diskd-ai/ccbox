import { base64ToBytes } from "@/domain/base64";
import { isRpcRejection } from "@/domain/errors";
import type { CcboxInfo, Engine, TimelineItem, TimelineItemKind } from "@/domain/types";

export type RpcCaller = <T = unknown>(method: string, params?: unknown) => Promise<T>;

export interface ProjectListItem {
  readonly path: string;
  readonly sessionCount: number | null;
  readonly lastSeenTs: string;
}

export interface SessionListItem {
  readonly sessionId: string;
  readonly engine: Engine;
  readonly startedTs: string;
  readonly title: string;
  readonly projectPath: string;
}

export interface TimelineSnapshot {
  readonly items: readonly TimelineItem[];
  readonly nextCursor: number | null;
}

export interface TimelineEvent {
  readonly sessionId: string;
  readonly cursor: number;
  readonly items: readonly TimelineItem[];
}

export interface TaskListItem {
  readonly taskId: string;
  readonly title: string;
  readonly projectPath: string;
  readonly updatedTs: string;
}

export interface TaskDetails {
  readonly taskId: string;
  readonly title: string;
  readonly body: string;
  readonly projectPath: string;
  readonly createdTs: string;
  readonly updatedTs: string;
}

export type ProcessStatusValue = "running" | "exited" | "error";

export interface ProcessListItem {
  readonly processId: string;
  readonly engine: Engine;
  readonly status: ProcessStatusValue;
  readonly startedTs: string;
  readonly projectPath: string;
  readonly sessionId: string | null;
  readonly exitCode: number | null;
}

export interface ProcessesLogEvent {
  readonly processId: string;
  readonly stream: "stdout" | "stderr" | "combined";
  readonly offset: number;
  readonly text: string;
}

function isObject(value: unknown): value is { [key: string]: unknown } {
  return typeof value === "object" && value !== null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function normalizeEngine(value: string): Engine {
  const lower = value.trim().toLowerCase();
  if (lower === "codex" || lower === "claude" || lower === "gemini" || lower === "opencode") {
    return lower;
  }
  return "codex";
}

function normalizeTimelineKind(value: string): TimelineItemKind {
  const lower = value.trim().toLowerCase();
  if (
    lower === "turn" ||
    lower === "user" ||
    lower === "assistant" ||
    lower === "thinking" ||
    lower === "tool_call" ||
    lower === "tool_output" ||
    lower === "token" ||
    lower === "token_count" ||
    lower === "note"
  ) {
    return lower;
  }
  return "note";
}

function parseTimelineItems(itemsValue: unknown): readonly TimelineItem[] {
  if (!Array.isArray(itemsValue)) return [];

  const parsed: TimelineItem[] = [];
  for (const item of itemsValue) {
    if (!isObject(item)) continue;

    const kindRaw = asString(item.kind);
    const summary = asString(item.summary);
    const detail = asString(item.detail) ?? "";
    if (!kindRaw || !summary) continue;

    const index = parsed.length + 1;
    const timestamp = asString(item.timestamp) ?? "";
    parsed.push({
      index,
      kind: normalizeTimelineKind(kindRaw),
      summary,
      detail,
      timestamp,
    });
  }

  return parsed;
}

function deriveTitleFromBody(body: string): string {
  const first = body
    .split("\n")
    .map((line) => line.trim())
    .find((line) => line.length > 0);

  if (!first) return "(untitled)";
  return first.length > 72 ? `${first.slice(0, 72)}...` : first;
}

export function formatTimestamp(value: string): string {
  if (!value) return "-";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

export function parseCcboxInfo(value: unknown): CcboxInfo | null {
  if (!isObject(value)) return null;

  const label = asString(value.label);
  const version = asString(value.version);
  const rawCapabilities = value.capabilities;

  if (!label || !version || !Array.isArray(rawCapabilities)) return null;

  const capabilities = rawCapabilities.filter((cap): cap is string => typeof cap === "string");

  return {
    label,
    version,
    capabilities,
    uptime_seconds: 0,
  };
}

export function parseProjectsListResult(result: unknown): readonly ProjectListItem[] {
  if (!isObject(result)) return [];
  if (!Array.isArray(result.projects)) return [];

  const rows: ProjectListItem[] = [];
  for (const project of result.projects) {
    if (!isObject(project)) continue;

    const path = asString(project.path);
    if (!path) continue;

    rows.push({
      path,
      sessionCount: asNumber(project.session_count),
      lastSeenTs: asString(project.last_seen_ts) ?? "",
    });
  }

  return rows;
}

export function parseSessionsListResult(result: unknown, projectPath: string): readonly SessionListItem[] {
  if (!isObject(result)) return [];
  if (!Array.isArray(result.sessions)) return [];

  const rows: SessionListItem[] = [];
  for (const session of result.sessions) {
    if (!isObject(session)) continue;

    const sessionId = asString(session.session_id);
    const engineRaw = asString(session.engine);
    if (!sessionId || !engineRaw) continue;

    rows.push({
      sessionId,
      engine: normalizeEngine(engineRaw),
      startedTs: asString(session.started_ts) ?? "",
      title: asString(session.title) ?? "",
      projectPath,
    });
  }

  return rows;
}

export function parseSessionsListAllResult(result: unknown): readonly SessionListItem[] {
  if (!isObject(result)) return [];
  if (!Array.isArray(result.sessions)) return [];

  const rows: SessionListItem[] = [];
  for (const session of result.sessions) {
    if (!isObject(session)) continue;

    const sessionId = asString(session.session_id);
    const engineRaw = asString(session.engine);
    const projectPath = asString(session.project_id);
    if (!sessionId || !engineRaw || !projectPath) continue;

    rows.push({
      sessionId,
      engine: normalizeEngine(engineRaw),
      startedTs: asString(session.started_ts) ?? "",
      title: asString(session.title) ?? "",
      projectPath,
    });
  }

  return rows;
}

export function parseTimelineSnapshot(result: unknown): TimelineSnapshot {
  if (!isObject(result)) {
    return { items: [], nextCursor: null };
  }

  return {
    items: parseTimelineItems(result.items),
    nextCursor: asNumber(result.next_cursor),
  };
}

export function parseTimelineEvent(data: unknown): TimelineEvent | null {
  if (!isObject(data)) return null;

  const sessionId = asString(data.session_id);
  const cursor = asNumber(data.cursor);
  if (!sessionId || cursor === null) return null;

  return {
    sessionId,
    cursor,
    items: parseTimelineItems(data.items),
  };
}

export function parseTasksListResult(result: unknown): readonly TaskListItem[] {
  if (!isObject(result)) return [];
  if (!Array.isArray(result.tasks)) return [];

  const rows: TaskListItem[] = [];
  for (const task of result.tasks) {
    if (!isObject(task)) continue;

    const taskId = asString(task.task_id);
    if (!taskId) continue;

    rows.push({
      taskId,
      title: asString(task.title) ?? "",
      projectPath: asString(task.project_path) ?? "",
      updatedTs: asString(task.updated_ts) ?? "",
    });
  }

  return rows;
}

export function parseTaskGetResult(result: unknown): TaskDetails | null {
  if (!isObject(result)) return null;
  if (!isObject(result.task)) return null;

  const taskId = asString(result.task.task_id);
  const body = asString(result.task.body);
  const projectPath = asString(result.task.project_path);
  if (!taskId || !body || !projectPath) return null;

  return {
    taskId,
    body,
    title: deriveTitleFromBody(body),
    projectPath,
    createdTs: asString(result.task.created_ts) ?? "",
    updatedTs: asString(result.task.updated_ts) ?? "",
  };
}

export function parseProcessesListResult(result: unknown): readonly ProcessListItem[] {
  if (!isObject(result)) return [];
  if (!Array.isArray(result.processes)) return [];

  const rows: ProcessListItem[] = [];
  for (const process of result.processes) {
    if (!isObject(process)) continue;

    const processId = asString(process.process_id);
    const engineRaw = asString(process.engine);
    if (!processId || !engineRaw) continue;

    const statusRaw = asString(process.status);
    const status: ProcessStatusValue =
      statusRaw === "running" || statusRaw === "exited" || statusRaw === "error" ? statusRaw : "error";

    rows.push({
      processId,
      engine: normalizeEngine(engineRaw),
      status,
      startedTs: asString(process.started_ts) ?? "",
      projectPath: asString(process.project_path) ?? "",
      sessionId: asString(process.session_id),
      exitCode: asNumber(process.exit_code),
    });
  }

  return rows;
}

export function parseProcessesLogEvent(data: unknown): ProcessesLogEvent | null {
  if (!isObject(data)) return null;

  const processId = asString(data.process_id);
  const stream = asString(data.stream);
  const offset = asNumber(data.offset);
  const chunkB64 = asString(data.chunk_b64);

  if (!processId || !stream || offset === null || !chunkB64) return null;
  if (stream !== "stdout" && stream !== "stderr" && stream !== "combined") return null;

  const bytes = base64ToBytes(chunkB64);
  const text = new TextDecoder().decode(bytes);

  return {
    processId,
    stream,
    offset,
    text,
  };
}

export async function fetchProjects(rpc: RpcCaller): Promise<readonly ProjectListItem[]> {
  const result = await rpc("projects.list", {});
  return parseProjectsListResult(result);
}

export async function fetchSessionsForProject(
  rpc: RpcCaller,
  projectPath: string,
): Promise<readonly SessionListItem[]> {
  const result = await rpc("sessions.list", { project_id: projectPath });
  return parseSessionsListResult(result, projectPath);
}

export async function fetchAllSessions(rpc: RpcCaller): Promise<readonly SessionListItem[]> {
  const sortSessions = (sessions: readonly SessionListItem[]): readonly SessionListItem[] => {
    return [...sessions].sort((a, b) => {
      const left = Date.parse(a.startedTs);
      const right = Date.parse(b.startedTs);
      if (Number.isNaN(left) || Number.isNaN(right)) return 0;
      return right - left;
    });
  };

  try {
    const result = await rpc("sessions.listAll", {});
    return sortSessions(parseSessionsListAllResult(result));
  } catch (reason: unknown) {
    if (isRpcRejection(reason) && reason.code === "UnsupportedCapability") {
      const projects = await fetchProjects(rpc);
      const all: SessionListItem[] = [];

      for (const project of projects) {
        const sessions = await fetchSessionsForProject(rpc, project.path);
        all.push(...sessions);
      }

      return sortSessions(all);
    }

    throw reason;
  }
}

export async function fetchSessionTimeline(
  rpc: RpcCaller,
  sessionId: string,
  limit: number,
  cursor: number,
): Promise<TimelineSnapshot> {
  const result = await rpc("sessions.getTimeline", {
    session_id: sessionId,
    limit,
    cursor,
  });
  return parseTimelineSnapshot(result);
}

export async function subscribeSessionTimeline(
  rpc: RpcCaller,
  sessionId: string,
  fromCursor?: number,
): Promise<number | null> {
  const result = await rpc("sessions.subscribeTimeline", {
    session_id: sessionId,
    ...(typeof fromCursor === "number" ? { from_cursor: fromCursor } : {}),
  });

  if (!isObject(result)) return null;
  return asNumber(result.cursor);
}

export async function fetchTasks(rpc: RpcCaller): Promise<readonly TaskListItem[]> {
  const result = await rpc("tasks.list", {});
  return parseTasksListResult(result);
}

export async function fetchTask(rpc: RpcCaller, taskId: string): Promise<TaskDetails | null> {
  const result = await rpc("tasks.get", { task_id: taskId });
  return parseTaskGetResult(result);
}

export async function createTask(
  rpc: RpcCaller,
  projectPath: string,
  body: string,
): Promise<string | null> {
  const result = await rpc("tasks.create", {
    project_path: projectPath,
    body,
    images: [],
  });

  if (!isObject(result)) return null;
  return asString(result.task_id);
}

export async function deleteTask(rpc: RpcCaller, taskId: string): Promise<boolean> {
  const result = await rpc("tasks.delete", { task_id: taskId });
  if (!isObject(result)) return false;
  return result.deleted === true;
}

export async function spawnTask(
  rpc: RpcCaller,
  taskId: string,
  engine: Engine,
): Promise<string | null> {
  const result = await rpc("tasks.spawn", {
    task_id: taskId,
    engine,
  });

  if (!isObject(result)) return null;
  return asString(result.process_id);
}

export async function fetchProcesses(rpc: RpcCaller): Promise<readonly ProcessListItem[]> {
  const result = await rpc("processes.list", {});
  return parseProcessesListResult(result);
}

export async function killProcess(rpc: RpcCaller, processId: string): Promise<boolean> {
  const result = await rpc("processes.kill", { process_id: processId });
  if (!isObject(result)) return false;
  return result.killed === true;
}

export async function subscribeProcessLogs(
  rpc: RpcCaller,
  processId: string,
  stream: "stdout" | "stderr" | "combined",
  fromOffset: number,
): Promise<number | null> {
  const result = await rpc("processes.subscribeLogs", {
    process_id: processId,
    stream,
    from_offset: fromOffset,
  });

  if (!isObject(result)) return null;
  return asNumber(result.offset);
}
