import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useParams, useNavigate } from "react-router-dom";
import { EngineBadge } from "@/components/StatusIndicators";
import type { Engine } from "@/components/StatusIndicators";
import { KIND_META, type TimelineItem, type TimelineItemKind } from "@/domain/types";
import {
  fetchAllSessions,
  fetchSessionTimeline,
  formatTimestamp,
  parseTimelineEvent,
  type SessionListItem,
} from "@/domain/remoteApi";
import { formatError } from "@/domain/errors";
import { useConnection } from "@/hooks/useConnection";
import { useSubscriptionStore } from "@/stores/subscriptionStore";
import { ChevronLeft, GripVertical, ChevronRight, AlertTriangle, BarChart3, Layers } from "lucide-react";

const KIND_COLORS: Record<TimelineItemKind, string> = {
  turn: "text-[hsl(var(--kind-turn))]",
  user: "text-[hsl(var(--kind-user))]",
  assistant: "text-foreground",
  thinking: "text-[hsl(var(--kind-thinking))] italic",
  tool_call: "text-[hsl(var(--kind-tool-call))]",
  tool_output: "text-[hsl(var(--kind-tool-output))]",
  token: "text-text-muted",
  token_count: "text-text-muted",
  note: "text-text-muted",
};

const KIND_BADGE_BG: Record<TimelineItemKind, string> = {
  turn: "bg-[hsl(var(--kind-turn)/0.15)] text-[hsl(var(--kind-turn))]",
  user: "bg-[hsl(var(--kind-user)/0.15)] text-[hsl(var(--kind-user))]",
  assistant: "bg-muted text-foreground",
  thinking: "bg-[hsl(var(--kind-thinking)/0.15)] text-[hsl(var(--kind-thinking))]",
  tool_call: "bg-[hsl(var(--kind-tool-call)/0.15)] text-[hsl(var(--kind-tool-call))]",
  tool_output: "bg-[hsl(var(--kind-tool-output)/0.15)] text-[hsl(var(--kind-tool-output))]",
  token: "bg-muted text-text-muted",
  token_count: "bg-muted text-text-muted",
  note: "bg-muted text-text-muted",
};

interface SessionMeta {
  readonly engine: Engine;
  readonly title: string;
  readonly started: string;
  readonly model: string;
  readonly cwd: string;
  readonly projectPath: string;
}

interface SkillSpanRow {
  readonly name: string;
  readonly startIndex: number;
  readonly endIndex: number;
  readonly depth: number;
  readonly loops: number;
}

function computeStats(timeline: readonly TimelineItem[]) {
  const toolCalls = timeline.filter((item) => item.kind === "tool_call");
  const toolOutputs = timeline.filter((item) => item.kind === "tool_output");
  const errors = toolOutputs.filter((item) => item.summary.toLowerCase().includes("error"));
  const tokenItem = timeline.find((item) => item.kind === "token" || item.kind === "token_count");

  const counts = new Map<string, number>();
  toolCalls.forEach((toolCall) => {
    const key = toolCall.summary.split(" ")[0] || "tool_call";
    counts.set(key, (counts.get(key) ?? 0) + 1);
  });

  const topTools = Array.from(counts.entries())
    .sort((left, right) => right[1] - left[1])
    .map(([name, count]) => ({ name, count }));

  return {
    totalItems: timeline.length,
    toolCallsTotal: toolCalls.length,
    toolCallsSuccess: Math.max(0, toolCalls.length - errors.length),
    toolCallsError: errors.length,
    topTools,
    tokenSummary: tokenItem?.summary ?? "-",
  };
}

function computeSkillSpans(timeline: readonly TimelineItem[]): readonly SkillSpanRow[] {
  const spans = new Map<string, { start: number; end: number; count: number }>();

  for (const item of timeline) {
    if (!item.skill_id) continue;
    const key = item.skill_id;
    const existing = spans.get(key);
    if (!existing) {
      spans.set(key, { start: item.index, end: item.index, count: 1 });
      continue;
    }

    spans.set(key, {
      start: Math.min(existing.start, item.index),
      end: Math.max(existing.end, item.index),
      count: existing.count + 1,
    });
  }

  return Array.from(spans.entries()).map(([name, row]) => ({
    name,
    startIndex: row.start,
    endIndex: row.end,
    depth: 0,
    loops: row.count > 1 ? row.count - 1 : 0,
  }));
}

type DetailTab = "timeline" | "skills" | "stats";

const SessionDetailPage = () => {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const sessionId = id ?? "";

  const { isConnected, rpc } = useConnection();
  const subscribe = useSubscriptionStore((state) => state.subscribe);
  const unsubscribe = useSubscriptionStore((state) => state.unsubscribe);
  const onEvent = useSubscriptionStore((state) => state.onEvent);

  const [meta, setMeta] = useState<SessionMeta>({
    engine: "codex",
    title: `Session ${sessionId}`,
    started: "",
    model: "unknown",
    cwd: "",
    projectPath: "",
  });
  const [timeline, setTimeline] = useState<readonly TimelineItem[]>([]);
  const [timelineCursor, setTimelineCursor] = useState<number>(0);
  const [timelineLoading, setTimelineLoading] = useState(false);
  const [timelineError, setTimelineError] = useState<string | null>(null);

  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [activeTab, setActiveTab] = useState<DetailTab>("timeline");
  const [splitRatio, setSplitRatio] = useState(0.4);
  const [mobileDetailOpen, setMobileDetailOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const isDragging = useRef(false);

  useEffect(() => {
    if (!isConnected || !sessionId) return;

    let cancelled = false;
    void fetchAllSessions(rpc)
      .then((sessions) => {
        if (cancelled) return;
        const found = sessions.find((session) => session.sessionId === sessionId);
        if (!found) {
          setMeta((prev) => ({ ...prev, title: `Session ${sessionId}` }));
          return;
        }

        setMeta({
          engine: found.engine,
          title: found.title || `Session ${sessionId}`,
          started: found.startedTs,
          model: "unknown",
          cwd: found.projectPath,
          projectPath: found.projectPath,
        });
      })
      .catch(() => {
        if (cancelled) return;
      });

    return () => {
      cancelled = true;
    };
  }, [isConnected, rpc, sessionId]);

  useEffect(() => {
    if (!isConnected || !sessionId) {
      setTimeline([]);
      setTimelineError(null);
      setTimelineLoading(false);
      return;
    }

    let cancelled = false;
    let subscriptionId: string | null = null;

    setTimelineLoading(true);
    setTimelineError(null);

    const stopListening = onEvent("sessions.timeline", (data) => {
      const event = parseTimelineEvent(data);
      if (!event) return;
      if (event.sessionId !== sessionId) return;

      setTimeline(event.items);
      setTimelineCursor(event.cursor);
    });

    void fetchSessionTimeline(rpc, sessionId, 200, 0)
      .then(async (snapshot) => {
        if (cancelled) return;

        setTimeline(snapshot.items);
        setTimelineCursor(snapshot.nextCursor ?? 0);

        subscriptionId = await subscribe({
          rpcMethod: "sessions.subscribeTimeline",
          eventTopic: "sessions.timeline",
          params:
            typeof snapshot.nextCursor === "number"
              ? { session_id: sessionId, from_cursor: snapshot.nextCursor }
              : { session_id: sessionId },
          cursorParam: "from_cursor",
          eventCursorField: "cursor",
        });
      })
      .catch((reason: unknown) => {
        if (cancelled) return;
        setTimelineError(formatError(reason));
      })
      .finally(() => {
        if (cancelled) return;
        setTimelineLoading(false);
      });

    return () => {
      cancelled = true;
      stopListening();
      if (subscriptionId) {
        unsubscribe(subscriptionId);
      }
    };
  }, [isConnected, rpc, sessionId, subscribe, unsubscribe, onEvent]);

  const stats = useMemo(() => computeStats(timeline), [timeline]);
  const skills = useMemo(() => computeSkillSpans(timeline), [timeline]);

  const selectedItem = selectedIndex !== null ? timeline.find((item) => item.index === selectedIndex) ?? null : null;

  const handleItemClick = (item: TimelineItem) => {
    setSelectedIndex(item.index);
    setMobileDetailOpen(true);
  };

  const handleMouseDown = useCallback(() => {
    isDragging.current = true;
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    const handleMouseMove = (event: MouseEvent) => {
      if (!isDragging.current || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const ratio = (event.clientX - rect.left) / rect.width;
      setSplitRatio(Math.max(0.2, Math.min(0.7, ratio)));
    };

    const handleMouseUp = () => {
      isDragging.current = false;
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      window.removeEventListener("mousemove", handleMouseMove);
      window.removeEventListener("mouseup", handleMouseUp);
    };

    window.addEventListener("mousemove", handleMouseMove);
    window.addEventListener("mouseup", handleMouseUp);
  }, []);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement;
      if (target.tagName === "INPUT" || target.tagName === "TEXTAREA") return;

      if (event.key === "j" || event.key === "ArrowDown") {
        event.preventDefault();
        setSelectedIndex((prev) => {
          const idx = prev ?? 0;
          const next = timeline.find((item) => item.index > idx);
          return next ? next.index : idx;
        });
      } else if (event.key === "k" || event.key === "ArrowUp") {
        event.preventDefault();
        setSelectedIndex((prev) => {
          const idx = prev ?? timeline.length;
          const items = timeline.filter((item) => item.index < idx);
          return items.length > 0 ? items[items.length - 1]?.index ?? prev : prev;
        });
      } else if (event.key === "Escape") {
        if (mobileDetailOpen) setMobileDetailOpen(false);
        else setSelectedIndex(null);
      } else if (event.key === "s") {
        setActiveTab("skills");
      } else if (event.key === "t") {
        setActiveTab("stats");
      } else if (event.key === "Backspace") {
        navigate(-1);
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [timeline, mobileDetailOpen, navigate]);

  return (
    <div className="flex flex-col h-full">
      <div className="border-b border-border px-4 py-2.5 bg-surface shrink-0 shadow-card">
        <div className="flex items-center gap-2 text-sm mb-1">
          <button onClick={() => navigate(-1)} className="text-text-secondary hover:text-foreground transition-colors">
            <ChevronLeft className="w-4 h-4" />
          </button>
          <span className="text-text-muted font-mono text-xs">{sessionId}</span>
          <span className="text-text-muted">|</span>
          <EngineBadge engine={meta.engine} />
          {meta.started && (
            <>
              <span className="text-text-muted">|</span>
              <span className="text-text-secondary">{formatTimestamp(meta.started)}</span>
            </>
          )}
          {meta.cwd && (
            <>
              <span className="text-text-muted hidden sm:inline">|</span>
              <span className="text-text-secondary hidden sm:inline truncate">{meta.cwd}</span>
            </>
          )}
        </div>
        <div className="flex items-center justify-between">
          <h1 className="text-lg font-sans font-semibold text-foreground">{meta.title}</h1>
          <div className="flex items-center gap-1">
            {(["timeline", "skills", "stats"] as const).map((tab) => (
              <button
                key={tab}
                onClick={() => setActiveTab(tab)}
                className={`px-3 py-1 text-xs font-sans font-medium transition-all rounded-lg capitalize flex items-center gap-1.5 ${
                  activeTab === tab
                    ? "bg-primary text-primary-foreground shadow-card"
                    : "text-text-secondary hover:text-foreground hover:bg-bg-hover"
                }`}
              >
                {tab === "skills" && <Layers className="w-3 h-3" />}
                {tab === "stats" && <BarChart3 className="w-3 h-3" />}
                {tab}
              </button>
            ))}
          </div>
        </div>
      </div>

      {activeTab === "timeline" && (
        <>
          {timelineLoading && (
            <div className="px-4 py-3 text-sm text-text-secondary border-b border-border">Loading timeline...</div>
          )}
          {timelineError && (
            <div className="px-4 py-3 text-sm text-status-error border-b border-border">{timelineError}</div>
          )}

          <div ref={containerRef} className="flex-1 hidden md:flex overflow-hidden">
            <div className="overflow-y-auto border-r border-border" style={{ width: `${splitRatio * 100}%` }}>
              <TimelineList items={timeline} selectedIndex={selectedIndex} onItemClick={handleItemClick} />
            </div>

            <div
              onMouseDown={handleMouseDown}
              className="w-1 shrink-0 cursor-col-resize hover:bg-primary/30 transition-colors flex items-center justify-center group"
            >
              <GripVertical className="w-3 h-3 text-text-muted opacity-0 group-hover:opacity-100 transition-opacity" />
            </div>

            <div className="overflow-y-auto flex-1">
              <DetailPane item={selectedItem} model={meta.model} cwd={meta.cwd} />
            </div>
          </div>

          <div className="flex-1 md:hidden overflow-y-auto">
            <TimelineList items={timeline} selectedIndex={selectedIndex} onItemClick={handleItemClick} />
          </div>

          {mobileDetailOpen && selectedItem && (
            <div className="fixed inset-0 z-30 md:hidden flex flex-col">
              <div className="flex-1 bg-background/60" onClick={() => setMobileDetailOpen(false)} />
              <div className="bg-surface border-t border-border rounded-t-2xl max-h-[70vh] overflow-y-auto animate-in slide-in-from-bottom shadow-elevated">
                <div className="flex justify-center py-2">
                  <div className="w-8 h-1 bg-text-muted rounded-full" />
                </div>
                <DetailPane item={selectedItem} model={meta.model} cwd={meta.cwd} />
              </div>
            </div>
          )}

          {!timelineLoading && timeline.length === 0 && (
            <div className="px-4 py-6 text-sm text-text-muted border-t border-border">No timeline items available.</div>
          )}
        </>
      )}

      {activeTab === "skills" && <SkillsPanel skills={skills} />}
      {activeTab === "stats" && <StatsPanel stats={stats} timelineCursor={timelineCursor} />}
    </div>
  );
};

function TimelineList({
  items,
  selectedIndex,
  onItemClick,
}: {
  items: readonly TimelineItem[];
  selectedIndex: number | null;
  onItemClick: (item: TimelineItem) => void;
}) {
  return (
    <div className="divide-y divide-border">
      {items.map((item) => {
        const meta = KIND_META[item.kind];
        const isSelected = item.index === selectedIndex;

        return (
          <div
            key={item.index}
            onClick={() => onItemClick(item)}
            className={`flex items-start gap-2 px-3 py-2 cursor-pointer transition-colors ${
              isSelected ? "bg-bg-active" : "hover:bg-bg-hover"
            } ${item.error ? "border-l-2 border-l-status-error" : ""}`}
          >
            <span className="text-text-muted text-xs font-mono w-5 shrink-0 text-right pt-0.5 select-none">{item.index}</span>

            <span className={`text-xs font-mono font-medium px-1.5 py-0.5 rounded shrink-0 ${KIND_BADGE_BG[item.kind]}`}>
              {meta.badge}
            </span>

            <span className={`text-sm truncate flex-1 ${KIND_COLORS[item.kind]}`}>{item.summary}</span>

            {isSelected && <ChevronRight className="w-3 h-3 text-text-muted shrink-0 mt-0.5" />}
          </div>
        );
      })}
    </div>
  );
}

function DetailPane({
  item,
  model,
  cwd,
}: {
  item: TimelineItem | null;
  model: string;
  cwd: string;
}) {
  if (!item) {
    return (
      <div className="flex items-center justify-center h-full text-text-muted text-sm p-8">
        <div className="text-center space-y-2">
          <p>Select a timeline item to view details</p>
          <p className="text-xs">
            Use <kbd className="bg-secondary px-1 py-0.5 rounded font-mono">j</kbd>/
            <kbd className="bg-secondary px-1 py-0.5 rounded font-mono">k</kbd> to navigate
          </p>
        </div>
      </div>
    );
  }

  const meta = KIND_META[item.kind];

  return (
    <div className="p-4 space-y-4">
      <div className="space-y-2">
        <div className="flex items-center gap-2">
          <span className={`text-xs font-mono font-medium px-1.5 py-0.5 rounded ${KIND_BADGE_BG[item.kind]}`}>{meta.badge}</span>
          <span className="text-sm font-sans font-medium text-foreground">{meta.label} #{item.index}</span>
        </div>

        {item.kind === "turn" && (
          <div className="bg-card border border-border rounded-xl p-3 space-y-1 text-sm shadow-card">
            <div className="flex items-center gap-2">
              <span className="text-text-secondary">Model:</span>
              <span className="text-foreground font-mono">{model}</span>
            </div>
            <div className="flex items-center gap-2">
              <span className="text-text-secondary">CWD:</span>
              <span className="text-foreground font-mono">{cwd || "-"}</span>
            </div>
          </div>
        )}
      </div>

      <div className="bg-card border border-border rounded-xl overflow-hidden shadow-card">
        <pre className="p-3 text-sm font-mono whitespace-pre-wrap break-words text-foreground leading-relaxed overflow-x-auto">
          {item.detail || item.summary}
        </pre>
      </div>
    </div>
  );
}

function SkillsPanel({ skills }: { skills: readonly SkillSpanRow[] }) {
  return (
    <div className="flex-1 overflow-y-auto p-4 lg:p-6 max-w-[1000px] mx-auto space-y-4">
      <h2 className="text-lg font-sans font-semibold text-foreground">Skills</h2>

      <div className="border border-border bg-surface rounded-xl shadow-card overflow-hidden">
        <div className="grid grid-cols-[1fr_80px_60px_80px] px-3 py-2 border-b border-border text-xs text-text-secondary uppercase tracking-wider font-sans">
          <span>Skill Name</span>
          <span className="text-right">Items</span>
          <span className="text-right">Depth</span>
          <span className="text-right">Loops</span>
        </div>

        {skills.map((skill) => (
          <div
            key={skill.name}
            className={`grid grid-cols-[1fr_80px_60px_80px] px-3 py-2.5 border-b border-border last:border-b-0 ${
              skill.loops > 0 ? "bg-[hsl(var(--status-warning)/0.1)]" : ""
            }`}
            style={{ paddingLeft: `${12 + skill.depth * 16}px` }}
          >
            <span className="text-foreground font-mono">{skill.name}</span>
            <span className="text-right text-text-secondary">
              {skill.startIndex}-{skill.endIndex}
            </span>
            <span className="text-right text-text-secondary">{skill.depth}</span>
            <span className="text-right">
              {skill.loops > 0 ? (
                <span className="text-status-warning flex items-center gap-1 justify-end">
                  <AlertTriangle className="w-3 h-3" />
                  {skill.loops}x
                </span>
              ) : (
                <span className="text-text-muted">-</span>
              )}
            </span>
          </div>
        ))}

        {skills.length === 0 && <div className="px-3 py-6 text-sm text-text-muted">No skill spans available.</div>}
      </div>

      <p className="text-xs text-text-muted">Depth 0 = top-level skill, depth 1+ = nested invocations.</p>
    </div>
  );
}

function StatsPanel({
  stats,
  timelineCursor,
}: {
  stats: ReturnType<typeof computeStats>;
  timelineCursor: number;
}) {
  const successPct =
    stats.toolCallsTotal > 0 ? Math.round((stats.toolCallsSuccess / stats.toolCallsTotal) * 100) : 0;
  const errorPct =
    stats.toolCallsTotal > 0 ? Math.round((stats.toolCallsError / stats.toolCallsTotal) * 100) : 0;

  return (
    <div className="flex-1 overflow-y-auto p-4 lg:p-6 max-w-[1000px] mx-auto space-y-6">
      <h2 className="text-lg font-sans font-semibold text-foreground">Session Statistics</h2>

      <div className="border border-border bg-surface rounded-xl shadow-card p-4 space-y-3">
        <h3 className="text-sm font-sans font-medium text-text-secondary uppercase tracking-wider">Tokens</h3>
        <div className="text-2xl font-sans font-bold text-foreground">{stats.tokenSummary}</div>
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card p-4 space-y-3">
        <h3 className="text-sm font-sans font-medium text-text-secondary uppercase tracking-wider">Tool Calls</h3>

        <div className="grid grid-cols-3 gap-4 text-center">
          <div>
            <div className="text-2xl font-sans font-bold text-foreground">{stats.toolCallsTotal}</div>
            <div className="text-xs text-text-secondary">Total</div>
          </div>
          <div>
            <div className="text-2xl font-sans font-bold text-status-online">{stats.toolCallsSuccess}</div>
            <div className="text-xs text-text-secondary">Success</div>
          </div>
          <div>
            <div className="text-2xl font-sans font-bold text-status-error">{stats.toolCallsError}</div>
            <div className="text-xs text-text-secondary">Error</div>
          </div>
        </div>

        <div className="h-2 bg-muted rounded-full overflow-hidden flex shadow-card">
          <div className="bg-status-online h-full" style={{ width: `${successPct}%` }} />
          <div className="bg-status-error h-full" style={{ width: `${errorPct}%` }} />
        </div>
        <div className="flex justify-between text-xs text-text-secondary">
          <span>{successPct}% success</span>
          <span>{errorPct}% error</span>
        </div>
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card p-4 space-y-3">
        <h3 className="text-sm font-sans font-medium text-text-secondary uppercase tracking-wider">Top Tools</h3>
        <div className="space-y-2">
          {stats.topTools.map((tool) => (
            <div key={tool.name} className="flex items-center justify-between">
              <span className="text-sm font-mono text-foreground">{tool.name}</span>
              <div className="flex items-center gap-2">
                <div
                  className="h-1.5 bg-primary rounded-full"
                  style={{
                    width: `${Math.max(
                      16,
                      stats.toolCallsTotal > 0 ? (tool.count / stats.toolCallsTotal) * 100 : 0,
                    )}px`,
                  }}
                />
                <span className="text-sm text-text-secondary w-4 text-right">{tool.count}</span>
              </div>
            </div>
          ))}
          {stats.topTools.length === 0 && <div className="text-sm text-text-muted">No tool calls yet.</div>}
        </div>
      </div>

      <div className="border border-border bg-surface rounded-xl shadow-card p-4 space-y-3">
        <h3 className="text-sm font-sans font-medium text-text-secondary uppercase tracking-wider">Timeline</h3>
        <div className="text-2xl font-sans font-bold text-foreground">{stats.totalItems} items</div>
        <div className="text-xs text-text-secondary">Cursor: {timelineCursor}</div>
      </div>
    </div>
  );
}

export default SessionDetailPage;
