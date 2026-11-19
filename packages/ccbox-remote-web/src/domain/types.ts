// Engine types
export type Engine = "codex" | "claude" | "gemini" | "opencode";

export const ENGINE_LIST: readonly Engine[] = ["codex", "claude", "gemini", "opencode"] as const;

export const ENGINE_META: Record<Engine, { label: string; short: string }> = {
  codex: { label: "Codex", short: "Cdx" },
  claude: { label: "Claude", short: "Cld" },
  gemini: { label: "Gemini", short: "Gem" },
  opencode: { label: "OpenCode", short: "OC" },
};

// Connection states
export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "awaiting_challenge"
  | "authenticating"
  | "online"
  | "reconnecting"
  | "auth_failed";

export interface CcboxInfo {
  readonly label: string;
  readonly version: string;
  readonly capabilities: readonly string[];
  readonly uptime_seconds: number;
}

// Projects
export interface Project {
  readonly path: string;
  readonly session_count: number | null;
  readonly last_seen_ts: string;
}

// Sessions
export interface Session {
  readonly session_id: string;
  readonly engine: Engine;
  readonly project_path: string;
  readonly title: string;
  readonly started_ts: string;
  readonly model?: string;
  readonly cwd?: string;
}

// Timeline
export type TimelineItemKind =
  | "turn"
  | "user"
  | "assistant"
  | "thinking"
  | "tool_call"
  | "tool_output"
  | "token"
  | "token_count"
  | "note";

export interface TimelineItem {
  readonly index: number;
  readonly kind: TimelineItemKind;
  readonly summary: string;
  readonly detail: string;
  readonly timestamp: string;
  readonly error?: boolean;
  readonly skill_id?: string;
}

export const KIND_META: Record<TimelineItemKind, { badge: string; label: string }> = {
  turn: { badge: "#", label: "Turn" },
  user: { badge: "U", label: "User" },
  assistant: { badge: "A", label: "Assistant" },
  thinking: { badge: "?", label: "Thinking" },
  tool_call: { badge: "T", label: "Tool Call" },
  tool_output: { badge: "O", label: "Tool Output" },
  token: { badge: "$", label: "Tokens" },
  token_count: { badge: "$", label: "Tokens" },
  note: { badge: "N", label: "Note" },
};

// Skills
export interface SkillSpan {
  readonly name: string;
  readonly start_index: number;
  readonly end_index: number;
  readonly depth: number;
  readonly loop_count: number;
}

// Session statistics
export interface SessionStats {
  readonly duration_seconds: number;
  readonly total_tokens: number;
  readonly input_tokens: number;
  readonly output_tokens: number;
  readonly tool_calls_total: number;
  readonly tool_calls_success: number;
  readonly tool_calls_error: number;
  readonly tool_calls_unknown: number;
  readonly top_tools: readonly { name: string; count: number }[];
  readonly files_modified: number;
  readonly lines_added: number;
  readonly lines_removed: number;
}

// Processes
export type ProcessStatus = "running" | "exited" | "error";

export interface Process {
  readonly id: string;
  readonly engine: Engine;
  readonly project_path: string;
  readonly started_at: string;
  readonly status: ProcessStatus;
  readonly exit_code?: number;
  readonly session_id?: string;
  readonly pid?: number;
}

export type LogStream = "stdout" | "stderr" | "combined";

export interface LogChunk {
  readonly stream: LogStream;
  readonly data: string;
  readonly offset: number;
}

// Tasks
export interface Task {
  readonly id: string;
  readonly title: string;
  readonly body: string;
  readonly project_path: string;
  readonly image_count: number;
  readonly updated_at: string;
}

export interface TaskCreateParams {
  readonly project_path: string;
  readonly body: string;
  readonly images?: readonly string[];
}

// Spawn
export type IoMode = "pipes" | "tty";

export interface SpawnParams {
  readonly engine: Engine;
  readonly project_path: string;
  readonly prompt: string;
  readonly io_mode: IoMode;
}

// Device identity
export interface DeviceIdentity {
  readonly kind: "webcrypto-ed25519-v1";
  readonly device_id: string;
  readonly created_at: string;
  readonly public_key_b64: string;
  readonly public_key: CryptoKey;
  readonly private_key: CryptoKey;
}
