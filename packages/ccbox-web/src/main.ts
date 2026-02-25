import { base64ToBytes } from "./base64";
import { loadOrCreateIdentity, resetIdentity } from "./identity";
import { connectRemoteClient } from "./protocol";

function getEl(id: string): HTMLElement {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing element: ${id}`);
  return node;
}

function getInput(id: string): HTMLInputElement {
  const node = getEl(id);
  if (!(node instanceof HTMLInputElement)) throw new Error(`Expected input: ${id}`);
  return node;
}

function getButton(id: string): HTMLButtonElement {
  const node = getEl(id);
  if (!(node instanceof HTMLButtonElement)) throw new Error(`Expected button: ${id}`);
  return node;
}

function getSelect(id: string): HTMLSelectElement {
  const node = getEl(id);
  if (!(node instanceof HTMLSelectElement)) throw new Error(`Expected select: ${id}`);
  return node;
}

function getTextarea(id: string): HTMLTextAreaElement {
  const node = getEl(id);
  if (!(node instanceof HTMLTextAreaElement)) throw new Error(`Expected textarea: ${id}`);
  return node;
}

function getPre(id: string): HTMLPreElement {
  const node = getEl(id);
  if (!(node instanceof HTMLPreElement)) throw new Error(`Expected pre: ${id}`);
  return node;
}

const statusLine = getEl("statusLine");
const connectInput = getInput("connectInput");
const connectBtn = getButton("connectBtn");
const refreshBtn = getButton("refreshBtn");
const disconnectBtn = getButton("disconnectBtn");
const resetIdentityBtn = getButton("resetIdentityBtn");
const pairBtn = getButton("pairBtn");
const deviceIdEl = getEl("deviceId");
const devicePubKeyEl = getEl("devicePubKey");
const lastErrorEl = getEl("lastError");
const projectsTbody = getEl("projectsTbody");
const rpcMeta = getEl("rpcMeta");
const pairCodeInput = getInput("pairCodeInput");
const pairStatusEl = getEl("pairStatus");

const engineSelect = getSelect("engineSelect");
const ioModeSelect = getSelect("ioModeSelect");
const projectPathInput = getInput("projectPathInput");
const promptInput = getTextarea("promptInput");
const spawnBtn = getButton("spawnBtn");
const killBtn = getButton("killBtn");
const spawnStatusEl = getEl("spawnStatus");
const logStreamSelect = getSelect("logStreamSelect");
const logOffsetEl = getEl("logOffset");
const subscribeLogsBtn = getButton("subscribeLogsBtn");
const clearLogsBtn = getButton("clearLogsBtn");
const logsPre = getPre("logsPre");

const sessionsListBtn = getButton("sessionsListBtn");
const sessionsTbody = getEl("sessionsTbody");
const sessionIdInput = getInput("sessionIdInput");
const timelineLimitInput = getInput("timelineLimitInput");
const timelineCursorEl = getEl("timelineCursor");
const timelineGetBtn = getButton("timelineGetBtn");
const timelineSubBtn = getButton("timelineSubBtn");
const timelineClearBtn = getButton("timelineClearBtn");
const timelinePre = getPre("timelinePre");

const tasksListBtn = getButton("tasksListBtn");
const tasksTbody = getEl("tasksTbody");
const taskIdInput = getInput("taskIdInput");
const taskBodyInput = getTextarea("taskBodyInput");
const taskCreateBtn = getButton("taskCreateBtn");
const taskDeleteBtn = getButton("taskDeleteBtn");
const taskSpawnBtn = getButton("taskSpawnBtn");
const taskStatusEl = getEl("taskStatus");

function setStatus(s: string) {
  statusLine.textContent = s;
}

function setError(s: string) {
  lastErrorEl.textContent = s;
}

function clearError() {
  setError("(none)");
}

function setPairStatus(s: string) {
  pairStatusEl.textContent = s;
}

function setSpawnStatus(s: string) {
  spawnStatusEl.textContent = s;
}

function setLogOffset(s: string) {
  logOffsetEl.textContent = s;
}

function setTimelineCursor(s: string) {
  timelineCursorEl.textContent = s;
}

function setTaskStatus(s: string) {
  taskStatusEl.textContent = s;
}

function isUuid(value: string): boolean {
  return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
}

function getRelayBase(): string | null {
  const raw = import.meta.env.VITE_CCBOX_RELAY_BASE;
  const value = typeof raw === "string" ? raw.trim() : "";
  return value ? value : null;
}

function buildRelayClientWsUrl(relayBase: string, guid: string): string {
  const v = relayBase.trim();
  const base = v.startsWith("wss://") || v.startsWith("ws://") ? v : `wss://${v}`;
  const u = new URL(base);
  if (u.protocol === "https:") u.protocol = "wss:";
  if (u.protocol === "http:") u.protocol = "ws:";
  u.pathname = "/client";
  u.searchParams.set("guid", guid);
  u.hash = "";
  return u.toString();
}

function isLikelyLocalHost(hostPort: string): boolean {
  const trimmed = hostPort.trim().toLowerCase();
  if (!trimmed) return false;

  const host = (() => {
    if (trimmed.startsWith("[")) {
      const end = trimmed.indexOf("]");
      return end > 0 ? trimmed.slice(1, end) : trimmed;
    }
    const idx = trimmed.indexOf(":");
    return idx >= 0 ? trimmed.slice(0, idx) : trimmed;
  })();

  if (host === "localhost") return true;
  if (host.endsWith(".local")) return true;

  const ipv4Parts = host.split(".");
  if (ipv4Parts.length === 4 && ipv4Parts.every((p) => /^\d+$/.test(p))) {
    const octets = ipv4Parts.map((p) => Number.parseInt(p, 10));
    if (octets.some((n) => Number.isNaN(n) || n < 0 || n > 255)) return false;
    const a = octets[0] ?? -1;
    const b = octets[1] ?? -1;
    if (a === 10) return true;
    if (a === 127) return true;
    if (a === 192 && b === 168) return true;
    if (a === 172 && b >= 16 && b <= 31) return true;
    if (a === 169 && b === 254) return true;
    return false;
  }

  if (host === "::1") return true;
  if (host.startsWith("fe80:")) return true;
  if (host.startsWith("fc") || host.startsWith("fd")) return true;

  return false;
}

function toWsUrl(raw: string): string {
  const v = raw.trim();
  if (!v) throw new Error("Missing connect value");
  if (v.startsWith("wss://") || v.startsWith("ws://")) {
    const u = new URL(v);
    if (!u.pathname || u.pathname === "/" || u.pathname.startsWith("/ccbox")) u.pathname = "/client";
    return u.toString();
  }

  if (v.startsWith("https://") || v.startsWith("http://")) {
    const u = new URL(v);
    u.protocol = u.protocol === "https:" ? "wss:" : "ws:";
    if (!u.pathname || u.pathname === "/") u.pathname = "/client";
    return u.toString();
  }

  if (isUuid(v)) {
    const relayBase = getRelayBase();
    if (relayBase) return buildRelayClientWsUrl(relayBase, v);
    return `wss://${v}.ccbox.app/client`;
  }

  const hostPort = v.split(/[/?#]/)[0] ?? "";
  const scheme = isLikelyLocalHost(hostPort) ? "ws" : "wss";

  if (/[/?#]/.test(v)) return `${scheme}://${v}`;
  return `${scheme}://${v}/client`;
}

function toPairUrl(raw: string): string {
  const wsUrl = toWsUrl(raw);
  const u = new URL(wsUrl);
  if (u.protocol === "ws:") u.protocol = "http:";
  else if (u.protocol === "wss:") u.protocol = "https:";
  else throw new Error("Unsupported URL scheme");
  u.pathname = "/pair";
  u.hash = "";
  return u.toString();
}

type ProjectRow = {
  readonly path: string;
  readonly sessionCount: number | null;
  readonly lastSeenIso: string;
};

type SessionRow = {
  readonly engine: string;
  readonly startedIso: string;
  readonly sessionId: string;
  readonly title: string;
};

type TimelineItemRow = {
  readonly kind: string;
  readonly timestampIso: string;
  readonly summary: string;
  readonly detail: string;
};

type TaskRow = {
  readonly updatedIso: string;
  readonly title: string;
  readonly taskId: string;
};

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function asString(value: unknown): string | null {
  return typeof value === "string" ? value : null;
}

function asNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function asIso(value: unknown): string {
  const s = asString(value);
  if (!s) return "";
  const d = new Date(s);
  return Number.isNaN(d.getTime()) ? "" : d.toISOString();
}

function parseProjectsListResult(result: unknown): readonly ProjectRow[] {
  if (!isObject(result)) return [];
  const projectsValue = result.projects;
  if (!Array.isArray(projectsValue)) return [];

  const rows: ProjectRow[] = [];
  for (const p of projectsValue) {
    if (!isObject(p)) continue;
    const path = asString(p.path);
    if (!path) continue;
    rows.push({
      path,
      sessionCount: asNumber(p.session_count),
      lastSeenIso: asIso(p.last_seen_ts),
    });
  }
  return rows;
}

function renderProjects(rows: readonly ProjectRow[]) {
  projectsTbody.textContent = "";

  if (rows.length === 0) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 3;
    td.className = "mono";
    td.style.color = "var(--muted)";
    td.textContent = "No projects returned.";
    tr.appendChild(td);
    projectsTbody.appendChild(tr);
    return;
  }

  for (const p of rows) {
    const tr = document.createElement("tr");
    tr.style.cursor = "pointer";
    tr.addEventListener("click", () => {
      projectPathInput.value = p.path;
      clearSessionsTable();
      sessionIdInput.value = "";
      clearTimeline();
      updateTimelineControls();
      updateTaskControls();
    });

    const tdPath = document.createElement("td");
    tdPath.className = "mono";
    tdPath.textContent = p.path;

    const tdCount = document.createElement("td");
    tdCount.className = "mono";
    tdCount.textContent = p.sessionCount === null ? "" : String(p.sessionCount);

    const tdLast = document.createElement("td");
    tdLast.className = "mono";
    tdLast.textContent = p.lastSeenIso;

    tr.appendChild(tdPath);
    tr.appendChild(tdCount);
    tr.appendChild(tdLast);
    projectsTbody.appendChild(tr);
  }
}

function parseSessionsListResult(result: unknown): readonly SessionRow[] {
  if (!isObject(result)) return [];
  const sessionsValue = result.sessions;
  if (!Array.isArray(sessionsValue)) return [];

  const rows: SessionRow[] = [];
  for (const s of sessionsValue) {
    if (!isObject(s)) continue;
    const engine = asString(s.engine);
    const sessionId = asString(s.session_id);
    if (!engine || !sessionId) continue;
    rows.push({
      engine,
      startedIso: asIso(s.started_ts),
      sessionId,
      title: asString(s.title) ?? "",
    });
  }
  return rows;
}

function renderSessions(rows: readonly SessionRow[], onPickSessionId: (sessionId: string) => void) {
  sessionsTbody.textContent = "";

  if (rows.length === 0) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 4;
    td.className = "mono";
    td.style.color = "var(--muted)";
    td.textContent = "No sessions returned.";
    tr.appendChild(td);
    sessionsTbody.appendChild(tr);
    return;
  }

  for (const s of rows) {
    const tr = document.createElement("tr");
    tr.style.cursor = "pointer";
    tr.addEventListener("click", () => onPickSessionId(s.sessionId));

    const tdEngine = document.createElement("td");
    tdEngine.className = "mono";
    tdEngine.textContent = s.engine;

    const tdStarted = document.createElement("td");
    tdStarted.className = "mono";
    tdStarted.textContent = s.startedIso;

    const tdId = document.createElement("td");
    tdId.className = "mono";
    tdId.textContent = s.sessionId;

    const tdTitle = document.createElement("td");
    tdTitle.textContent = s.title;

    tr.appendChild(tdEngine);
    tr.appendChild(tdStarted);
    tr.appendChild(tdId);
    tr.appendChild(tdTitle);
    sessionsTbody.appendChild(tr);
  }
}

function parseTimelineItem(value: unknown): TimelineItemRow | null {
  if (!isObject(value)) return null;
  const kind = asString(value.kind);
  const summary = asString(value.summary);
  if (!kind || !summary) return null;
  return {
    kind,
    timestampIso: asIso(value.timestamp),
    summary,
    detail: asString(value.detail) ?? "",
  };
}

function formatTimelineItem(item: TimelineItemRow): string {
  const ts = item.timestampIso ? `[${item.timestampIso}]` : "";
  const head = `${ts} ${item.kind}: ${item.summary}`.trim();
  return head;
}

function parseTasksListResult(result: unknown): readonly TaskRow[] {
  if (!isObject(result)) return [];
  const tasksValue = result.tasks;
  if (!Array.isArray(tasksValue)) return [];

  const rows: TaskRow[] = [];
  for (const t of tasksValue) {
    if (!isObject(t)) continue;
    const taskId = asString(t.task_id);
    if (!taskId) continue;
    rows.push({
      taskId,
      title: asString(t.title) ?? "",
      updatedIso: asIso(t.updated_ts),
    });
  }
  return rows;
}

function renderTasks(rows: readonly TaskRow[], onPickTaskId: (taskId: string) => void) {
  tasksTbody.textContent = "";

  if (rows.length === 0) {
    const tr = document.createElement("tr");
    const td = document.createElement("td");
    td.colSpan = 3;
    td.className = "mono";
    td.style.color = "var(--muted)";
    td.textContent = "No tasks returned.";
    tr.appendChild(td);
    tasksTbody.appendChild(tr);
    return;
  }

  for (const t of rows) {
    const tr = document.createElement("tr");
    tr.style.cursor = "pointer";
    tr.addEventListener("click", () => onPickTaskId(t.taskId));

    const tdUpdated = document.createElement("td");
    tdUpdated.className = "mono";
    tdUpdated.textContent = t.updatedIso;

    const tdTitle = document.createElement("td");
    tdTitle.textContent = t.title;

    const tdId = document.createElement("td");
    tdId.className = "mono";
    tdId.textContent = t.taskId;

    tr.appendChild(tdUpdated);
    tr.appendChild(tdTitle);
    tr.appendChild(tdId);
    tasksTbody.appendChild(tr);
  }
}

let client: Awaited<ReturnType<typeof connectRemoteClient>> | null = null;
let currentProcessId: string | null = null;
let subscribedStream: string | null = null;
let lastLogOffset: number | null = null;
let timelineCursorBytes: number | null = null;

const textDecoder = new TextDecoder();

function clearLogs() {
  logsPre.textContent = "";
  lastLogOffset = null;
  setLogOffset("(not subscribed)");
}

function appendLogs(text: string) {
  logsPre.textContent = `${logsPre.textContent ?? ""}${text}`;
  logsPre.scrollTop = logsPre.scrollHeight;
}

function setCurrentProcess(processId: string | null) {
  const changed = currentProcessId !== processId;
  currentProcessId = processId;
  killBtn.disabled = !currentProcessId;
  subscribeLogsBtn.disabled = !currentProcessId;
  clearLogsBtn.disabled = !currentProcessId;
  if (!currentProcessId) {
    setSpawnStatus("(no process)");
    subscribedStream = null;
    clearLogs();
    return;
  }

  if (changed) {
    subscribedStream = null;
    clearLogs();
  }
}

function clearTimeline() {
  timelinePre.textContent = "";
  timelineCursorBytes = null;
  setTimelineCursor("(none)");
}

function renderTimeline(items: readonly TimelineItemRow[]) {
  timelinePre.textContent = items.map(formatTimelineItem).join("\n");
  timelinePre.scrollTop = timelinePre.scrollHeight;
}

function updateTimelineControls() {
  const sessionId = sessionIdInput.value.trim();
  const enabled = Boolean(client) && Boolean(sessionId);
  timelineGetBtn.disabled = !enabled;
  timelineSubBtn.disabled = !enabled;
  timelineClearBtn.disabled = !(enabled && (timelinePre.textContent ?? "").length > 0);
}

function updateTaskControls() {
  const taskId = taskIdInput.value.trim();
  const connected = Boolean(client);
  const canCreate =
    connected && Boolean(projectPathInput.value.trim()) && Boolean(taskBodyInput.value.trim());
  tasksListBtn.disabled = !connected;
  taskCreateBtn.disabled = !canCreate;
  taskDeleteBtn.disabled = !(connected && taskId);
  taskSpawnBtn.disabled = !(connected && taskId);
}

async function pairDevice(): Promise<boolean> {
  clearError();
  setPairStatus("pairing…");

  const code = pairCodeInput.value.trim();
  if (!code) {
    setPairStatus("pair failed");
    setError("Missing pairing code");
    return false;
  }

  let pairUrl: string;
  try {
    pairUrl = toPairUrl(connectInput.value);
  } catch (err) {
    setPairStatus("pair failed");
    setError(String(err));
    return false;
  }

  try {
    const identity = await loadOrCreateIdentity();
    const res = await fetch(pairUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        pairing_code: code,
        device_id: identity.deviceId,
        public_key_b64: identity.publicKeyRawB64,
      }),
    });

    const data: unknown = await res.json();
    if (isObject(data) && data.ok === true) {
      setPairStatus("paired");
      return true;
    }

    const err = isObject(data) ? asString(data.error) : null;
    setPairStatus("pair failed");
    setError(err ?? `Pair failed (HTTP ${res.status})`);
    return false;
  } catch (err) {
    setPairStatus("pair error");
    setError(String(err));
    return false;
  }
}

function parseProcessId(result: unknown): string | null {
  if (!isObject(result)) return null;
  const pid = asString(result.process_id);
  return pid ?? null;
}

type ProcessesLogEvent = {
  readonly processId: string;
  readonly stream: string;
  readonly offset: number;
  readonly chunkB64: string;
};

function parseProcessesLogEvent(data: unknown): ProcessesLogEvent | null {
  if (!isObject(data)) return null;
  const processId = asString(data.process_id);
  const stream = asString(data.stream);
  const offset = asNumber(data.offset);
  const chunkB64 = asString(data.chunk_b64);
  if (!processId || !stream || offset === null || !chunkB64) return null;
  return { processId, stream, offset, chunkB64 };
}

type SessionsTimelineEvent = {
  readonly sessionId: string;
  readonly cursor: number;
  readonly items: readonly TimelineItemRow[];
};

function parseSessionsTimelineEvent(data: unknown): SessionsTimelineEvent | null {
  if (!isObject(data)) return null;
  const sessionId = asString(data.session_id);
  const cursor = asNumber(data.cursor);
  const itemsValue = data.items;
  if (!sessionId || cursor === null || !Array.isArray(itemsValue)) return null;

  const items: TimelineItemRow[] = [];
  for (const item of itemsValue) {
    const parsed = parseTimelineItem(item);
    if (parsed) items.push(parsed);
  }

  return { sessionId, cursor, items };
}

function handleEvent(topic: string, data: unknown) {
  if (topic === "processes.log") {
    const ev = parseProcessesLogEvent(data);
    if (!ev) return;
    if (!currentProcessId) return;
    if (ev.processId !== currentProcessId) return;
    if (subscribedStream && ev.stream !== subscribedStream) return;

    const bytes = base64ToBytes(ev.chunkB64);
    const text = textDecoder.decode(bytes);
    appendLogs(text);
    lastLogOffset = ev.offset;
    setLogOffset(String(ev.offset));
    return;
  }

  if (topic === "sessions.timeline") {
    const ev = parseSessionsTimelineEvent(data);
    if (!ev) return;
    const selected = sessionIdInput.value.trim();
    if (!selected || selected !== ev.sessionId) return;
    timelineCursorBytes = ev.cursor;
    setTimelineCursor(String(ev.cursor));
    renderTimeline(ev.items);
    updateTimelineControls();
  }
}

async function spawnAgent() {
  if (!client) return;
  clearError();

  const projectPath = projectPathInput.value.trim();
  const prompt = promptInput.value;
  if (!projectPath) {
    setError("Missing project path");
    return;
  }
  if (!prompt.trim()) {
    setError("Missing prompt");
    return;
  }

  clearLogs();
  setSpawnStatus("spawning…");
  setCurrentProcess(null);

  try {
    const result = await client.rpc("agents.spawn", {
      engine: engineSelect.value,
      project_path: projectPath,
      prompt,
      io_mode: ioModeSelect.value,
    });

    const pid = parseProcessId(result);
    if (!pid) {
      setSpawnStatus("spawn error");
      setError("Invalid agents.spawn response (missing process_id)");
      return;
    }

    setCurrentProcess(pid);
    setSpawnStatus(`process_id=${pid}`);
    await subscribeLogs();
  } catch (err) {
    setSpawnStatus("spawn error");
    setError(String(err));
  }
}

async function subscribeLogs() {
  if (!client) return;
  if (!currentProcessId) return;
  clearError();

  const stream = logStreamSelect.value;
  subscribedStream = stream;
  setLogOffset("subscribing…");

  try {
    await client.rpc("processes.subscribeLogs", {
      process_id: currentProcessId,
      stream,
      from_offset: lastLogOffset ?? 0,
    });
    setLogOffset(String(lastLogOffset ?? 0));
  } catch (err) {
    setLogOffset("subscribe error");
    setError(String(err));
  }
}

async function killProcess() {
  if (!client) return;
  if (!currentProcessId) return;
  clearError();
  try {
    await client.rpc("processes.kill", { process_id: currentProcessId });
  } catch (err) {
    setError(String(err));
  }
}

function clearSessionsTable() {
  sessionsTbody.textContent = "";
  const tr = document.createElement("tr");
  const td = document.createElement("td");
  td.colSpan = 4;
  td.className = "mono";
  td.style.color = "var(--muted)";
  td.textContent = "No sessions loaded.";
  tr.appendChild(td);
  sessionsTbody.appendChild(tr);
}

function clearTasksTable() {
  tasksTbody.textContent = "";
  const tr = document.createElement("tr");
  const td = document.createElement("td");
  td.colSpan = 3;
  td.className = "mono";
  td.style.color = "var(--muted)";
  td.textContent = "No tasks loaded.";
  tr.appendChild(td);
  tasksTbody.appendChild(tr);
}

function setSelectedSessionId(sessionId: string) {
  sessionIdInput.value = sessionId;
  clearTimeline();
  updateTimelineControls();
}

async function refreshSessions() {
  if (!client) return;
  clearError();

  const projectId = projectPathInput.value.trim();
  if (!projectId) {
    setError("Missing project path (used as project_id for sessions.list)");
    return;
  }

  try {
    const result = await client.rpc("sessions.list", { project_id: projectId });
    const rows = parseSessionsListResult(result);
    renderSessions(rows, (sid) => setSelectedSessionId(sid));
    updateTimelineControls();
  } catch (err) {
    setError(String(err));
  }
}

function parseTimelineSnapshot(result: unknown): { readonly items: readonly TimelineItemRow[]; readonly nextCursor: number | null } {
  if (!isObject(result)) return { items: [], nextCursor: null };
  const nextCursor = asNumber(result.next_cursor);
  const itemsValue = result.items;
  if (!Array.isArray(itemsValue)) return { items: [], nextCursor };

  const items: TimelineItemRow[] = [];
  for (const item of itemsValue) {
    const parsed = parseTimelineItem(item);
    if (parsed) items.push(parsed);
  }

  return { items, nextCursor };
}

function parseTimelineLimit(): number {
  const raw = timelineLimitInput.value.trim();
  const n = Number.parseInt(raw, 10);
  if (!Number.isFinite(n)) return 200;
  return Math.min(1000, Math.max(1, n));
}

async function getTimeline() {
  if (!client) return;
  clearError();

  const sessionId = sessionIdInput.value.trim();
  if (!sessionId) {
    setError("Missing session_id");
    return;
  }

  try {
    const result = await client.rpc("sessions.getTimeline", {
      session_id: sessionId,
      limit: parseTimelineLimit(),
      cursor: timelineCursorBytes ?? 0,
    });

    const parsed = parseTimelineSnapshot(result);
    if (parsed.nextCursor !== null) {
      timelineCursorBytes = parsed.nextCursor;
      setTimelineCursor(String(parsed.nextCursor));
    }

    if (parsed.items.length > 0) {
      renderTimeline(parsed.items);
    }
    updateTimelineControls();
  } catch (err) {
    setError(String(err));
  }
}

async function subscribeTimeline() {
  if (!client) return;
  clearError();

  const sessionId = sessionIdInput.value.trim();
  if (!sessionId) {
    setError("Missing session_id");
    return;
  }

  try {
    const result = await client.rpc("sessions.subscribeTimeline", {
      session_id: sessionId,
      from_cursor: timelineCursorBytes ?? undefined,
    });
    if (isObject(result)) {
      const cursor = asNumber(result.cursor);
      if (cursor !== null) {
        timelineCursorBytes = cursor;
        setTimelineCursor(String(cursor));
      }
    }
    updateTimelineControls();
  } catch (err) {
    setError(String(err));
  }
}

function setSelectedTaskId(taskId: string) {
  taskIdInput.value = taskId;
  updateTaskControls();
}

async function refreshTasks() {
  if (!client) return;
  clearError();
  setTaskStatus("tasks.list (loading)");

  try {
    const result = await client.rpc("tasks.list", {});
    const rows = parseTasksListResult(result);
    renderTasks(rows, (id) => setSelectedTaskId(id));
    setTaskStatus("tasks.list (ok)");
    updateTaskControls();
  } catch (err) {
    setTaskStatus("tasks.list (error)");
    setError(String(err));
  }
}

async function createTask() {
  if (!client) return;
  clearError();

  const projectPath = projectPathInput.value.trim();
  const body = taskBodyInput.value.trim();
  if (!projectPath) {
    setError("Missing project path (used as project_path for tasks.create)");
    return;
  }
  if (!body) {
    setError("Missing task body");
    return;
  }

  setTaskStatus("tasks.create (loading)");
  try {
    const result = await client.rpc("tasks.create", {
      project_path: projectPath,
      body,
      images: [],
    });
    const taskId = isObject(result) ? asString(result.task_id) : null;
    if (taskId) {
      setSelectedTaskId(taskId);
    }
    taskBodyInput.value = "";
    setTaskStatus("tasks.create (ok)");
    updateTaskControls();
    await refreshTasks();
  } catch (err) {
    setTaskStatus("tasks.create (error)");
    setError(String(err));
  }
}

async function deleteTask() {
  if (!client) return;
  clearError();

  const taskId = taskIdInput.value.trim();
  if (!taskId) {
    setError("Missing task_id");
    return;
  }

  setTaskStatus("tasks.delete (loading)");
  try {
    await client.rpc("tasks.delete", { task_id: taskId });
    taskIdInput.value = "";
    setTaskStatus("tasks.delete (ok)");
    updateTaskControls();
    await refreshTasks();
  } catch (err) {
    setTaskStatus("tasks.delete (error)");
    setError(String(err));
  }
}

async function spawnTask() {
  if (!client) return;
  clearError();

  const taskId = taskIdInput.value.trim();
  if (!taskId) {
    setError("Missing task_id");
    return;
  }

  setTaskStatus("tasks.spawn (loading)");
  try {
    const result = await client.rpc("tasks.spawn", { task_id: taskId, engine: engineSelect.value });
    const pid = parseProcessId(result);
    if (!pid) {
      setTaskStatus("tasks.spawn (error)");
      setError("Invalid tasks.spawn response (missing process_id)");
      return;
    }

    setTaskStatus(`tasks.spawn (ok) process_id=${pid}`);
    setCurrentProcess(pid);
    setSpawnStatus(`process_id=${pid}`);
    await subscribeLogs();
  } catch (err) {
    setTaskStatus("tasks.spawn (error)");
    setError(String(err));
  }
}

async function refreshProjects() {
  if (!client) return;
  clearError();
  rpcMeta.textContent = "projects.list (loading)";

  try {
    const result = await client.rpc("projects.list", {});
    renderProjects(parseProjectsListResult(result));
    rpcMeta.textContent = "projects.list (ok)";
  } catch (err) {
    rpcMeta.textContent = "projects.list (error)";
    setError(String(err));
  }
}

async function connect() {
  clearError();
  if (client) client.close();
  client = null;
  spawnBtn.disabled = true;
  sessionsListBtn.disabled = true;
  tasksListBtn.disabled = true;
  setCurrentProcess(null);
  clearSessionsTable();
  sessionIdInput.value = "";
  clearTimeline();
  updateTimelineControls();
  clearTasksTable();
  taskIdInput.value = "";
  taskBodyInput.value = "";
  setTaskStatus("(no tasks)");
  updateTaskControls();
  refreshBtn.disabled = true;
  disconnectBtn.disabled = true;
  rpcMeta.textContent = "connecting…";

  const wsUrl = toWsUrl(connectInput.value);
  const identity = await loadOrCreateIdentity();

  client = await connectRemoteClient(wsUrl, identity, {
    onStatus: (s) => setStatus(s),
    onError: (e) => setError(e),
    onEvent: (topic, data) => handleEvent(topic, data),
  });

  refreshBtn.disabled = false;
  disconnectBtn.disabled = false;
  spawnBtn.disabled = false;
  sessionsListBtn.disabled = false;
  updateTimelineControls();
  updateTaskControls();
  rpcMeta.textContent = `connected (${wsUrl})`;
  await refreshProjects();
}

function disconnect() {
  clearError();
  if (client) client.close();
  client = null;
  refreshBtn.disabled = true;
  disconnectBtn.disabled = true;
  spawnBtn.disabled = true;
  sessionsListBtn.disabled = true;
  tasksListBtn.disabled = true;
  setCurrentProcess(null);
  clearSessionsTable();
  sessionIdInput.value = "";
  clearTimeline();
  updateTimelineControls();
  clearTasksTable();
  taskIdInput.value = "";
  taskBodyInput.value = "";
  setTaskStatus("(no tasks)");
  updateTaskControls();
  rpcMeta.textContent = "not connected";
  setStatus("disconnected");
}

async function init() {
  try {
    const identity = await loadOrCreateIdentity();
    deviceIdEl.textContent = identity.deviceId;
    devicePubKeyEl.textContent = identity.publicKeyRawB64;
    setStatus("ready");
    setPairStatus("(not paired)");
  } catch (err) {
    setStatus("identity error");
    setError(String(err));
    connectBtn.disabled = true;
    pairBtn.disabled = true;
    return;
  }

  const url = new URL(window.location.href);
  const connectParam = url.searchParams.get("connect");
  if (connectParam && connectParam.trim()) connectInput.value = connectParam.trim();
  const pairingCodeParam = url.searchParams.get("pairing_code");
  if (pairingCodeParam && pairingCodeParam.trim()) pairCodeInput.value = pairingCodeParam.trim();
  const guidParam = url.searchParams.get("guid");
  if (!connectInput.value && guidParam && isUuid(guidParam)) connectInput.value = guidParam;

  const host = window.location.hostname;
  if (host.endsWith(".ccbox.app")) {
    const guid = host.slice(0, -".ccbox.app".length);
    if (!connectInput.value && isUuid(guid)) connectInput.value = guid;
  }

  const autoConnectRaw = (url.searchParams.get("autoconnect") ?? "").trim().toLowerCase();
  const autoConnectExplicit = autoConnectRaw === "1" || autoConnectRaw === "true";
  const autoConnectDisabled = autoConnectRaw === "0" || autoConnectRaw === "false";
  const shouldAutoConnect = !autoConnectDisabled && (autoConnectExplicit || Boolean(connectParam?.trim()));
  const hasConnect = Boolean(connectInput.value.trim());
  const hasPairingCode = Boolean(pairCodeInput.value.trim());

  if (shouldAutoConnect && hasConnect) {
    if (hasPairingCode) {
      const paired = await pairDevice();
      if (!paired) return;
    }
    await connect();
  }
}

connectBtn.addEventListener("click", () => {
  connect().catch((err) => setError(String(err)));
});

spawnBtn.addEventListener("click", () => {
  spawnAgent().catch((err) => setError(String(err)));
});

subscribeLogsBtn.addEventListener("click", () => {
  subscribeLogs().catch((err) => setError(String(err)));
});

killBtn.addEventListener("click", () => {
  killProcess().catch((err) => setError(String(err)));
});

clearLogsBtn.addEventListener("click", () => {
  clearLogs();
});

sessionsListBtn.addEventListener("click", () => {
  refreshSessions().catch((err) => setError(String(err)));
});

sessionIdInput.addEventListener("input", () => {
  clearTimeline();
  updateTimelineControls();
});

timelineGetBtn.addEventListener("click", () => {
  getTimeline().catch((err) => setError(String(err)));
});

timelineSubBtn.addEventListener("click", () => {
  subscribeTimeline().catch((err) => setError(String(err)));
});

timelineClearBtn.addEventListener("click", () => {
  clearTimeline();
  updateTimelineControls();
});

projectPathInput.addEventListener("input", () => {
  updateTaskControls();
});

tasksListBtn.addEventListener("click", () => {
  refreshTasks().catch((err) => setError(String(err)));
});

taskIdInput.addEventListener("input", () => {
  updateTaskControls();
});

taskBodyInput.addEventListener("input", () => {
  updateTaskControls();
});

taskCreateBtn.addEventListener("click", () => {
  createTask().catch((err) => setError(String(err)));
});

taskDeleteBtn.addEventListener("click", () => {
  deleteTask().catch((err) => setError(String(err)));
});

taskSpawnBtn.addEventListener("click", () => {
  spawnTask().catch((err) => setError(String(err)));
});

pairBtn.addEventListener("click", () => {
  pairDevice().catch((err) => setError(String(err)));
});

refreshBtn.addEventListener("click", () => {
  refreshProjects().catch((err) => setError(String(err)));
});

disconnectBtn.addEventListener("click", () => {
  disconnect();
});

resetIdentityBtn.addEventListener("click", () => {
  resetIdentity()
    .then(() => window.location.reload())
    .catch((err) => setError(String(err)));
});

init().catch((err) => setError(String(err)));
