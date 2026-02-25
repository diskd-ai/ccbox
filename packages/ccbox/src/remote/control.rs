use crate::domain::{AgentEngine, SpawnIoMode};
use crate::infra::{ProcessManager, ProcessSignal, SpawnedAgentIo};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

pub struct ControlPlane {
    sessions_dir: PathBuf,
    process_manager: ProcessManager,
    process_signal_rx: std::sync::mpsc::Receiver<ProcessSignal>,
    processes: HashMap<String, ProcessEntry>,
    log_subscriptions: HashMap<String, LogSubscription>,
    timeline_subscriptions: HashMap<String, TimelineSubscription>,
}

struct ProcessEntry {
    process: crate::infra::SpawnedAgentProcess,
    status: ProcessStatus,
    exit_code: Option<i32>,
    session_id: Option<String>,
    session_log_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProcessStatus {
    Running,
    Exited,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogStream {
    Stdout,
    Stderr,
    Combined,
}

struct LogSubscription {
    session_id: String,
    process_id: String,
    stream: LogStream,
    offset: u64,
}

struct TimelineSubscription {
    session_id: String,
    target_session_id: String,
    log_path: PathBuf,
    cursor_bytes: u64,
    limit: usize,
    next_poll_at: Instant,
    poll_every: Duration,
}

impl ControlPlane {
    pub fn new(sessions_dir: PathBuf) -> Result<Self, super::ServeError> {
        let (tx, rx) = std::sync::mpsc::channel::<ProcessSignal>();
        let process_manager = ProcessManager::new(sessions_dir.clone(), tx)
            .map_err(|error| super::ServeError::ProcessManager(error.to_string()))?;

        Ok(Self {
            sessions_dir,
            process_manager,
            process_signal_rx: rx,
            processes: HashMap::new(),
            log_subscriptions: HashMap::new(),
            timeline_subscriptions: HashMap::new(),
        })
    }

    pub fn on_connected(&mut self) {
        self.log_subscriptions.clear();
        self.timeline_subscriptions.clear();
    }

    pub async fn tick(&mut self, ws: &mut super::WsStream) -> Result<(), super::ServeError> {
        self.drain_process_signals();
        self.drain_process_exits();
        self.drain_log_subscriptions(ws).await?;
        self.drain_timeline_subscriptions(ws).await?;
        Ok(())
    }

    pub async fn handle_rpc(
        &mut self,
        ws: &mut super::WsStream,
        session_id: String,
        opts: &super::ServeOptions,
        connection_guid: &str,
        req: &super::RpcRequestPayload,
    ) -> Result<(), super::ServeError> {
        let result = self
            .dispatch_rpc(opts, connection_guid, &session_id, req)
            .await;
        let id = req.id.clone();

        let response_payload = match result {
            Ok(value) => serde_json::json!({
                "id": id.clone(),
                "ok": true,
                "result": value,
            }),
            Err(error) => serde_json::json!({
                "id": id.clone(),
                "ok": false,
                "error": { "code": error.code, "message": error.message },
            }),
        };

        let response_env = super::EnvelopeOut {
            v: super::REMOTE_PROTOCOL_VERSION,
            type_: "rpc/response",
            ts: super::now_iso(),
            payload: response_payload,
        };

        let response_text = serde_json::to_string(&response_env)?;

        let out_frame = super::EnvelopeOut {
            v: super::REMOTE_PROTOCOL_VERSION,
            type_: "mux/frame",
            ts: super::now_iso(),
            payload: super::MuxFramePayloadOut {
                session_id,
                stream_id: super::CONTROL_V1_STREAM_ID,
                payload_b64: base64::engine::general_purpose::STANDARD
                    .encode(response_text.as_bytes()),
            },
        };

        super::send_json(ws, &out_frame).await
    }

    async fn dispatch_rpc(
        &mut self,
        opts: &super::ServeOptions,
        connection_guid: &str,
        session_id: &str,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        match req.method.as_str() {
            "projects.list" => self.handle_projects_list().await,
            "sessions.list" => self.handle_sessions_list(req).await,
            "sessions.getTimeline" => self.handle_sessions_get_timeline(req).await,
            "sessions.subscribeTimeline" => {
                self.handle_sessions_subscribe_timeline(session_id, req)
                    .await
            }
            "tasks.list" => self.handle_tasks_list().await,
            "tasks.get" => self.handle_tasks_get(req).await,
            "tasks.create" => self.handle_tasks_create(req).await,
            "tasks.delete" => self.handle_tasks_delete(req).await,
            "tasks.spawn" => self.handle_tasks_spawn(req).await,
            "ccbox.getInfo" => Ok(self.handle_get_info(opts, connection_guid)),
            "agents.spawn" => self.handle_agents_spawn(req).await,
            "processes.list" => Ok(self.handle_processes_list()),
            "processes.kill" => self.handle_processes_kill(req).await,
            "processes.subscribeLogs" => {
                self.handle_processes_subscribe_logs(session_id, req).await
            }
            _ => Err(RpcMethodError {
                code: "UnsupportedCapability".to_string(),
                message: format!("unsupported method: {}", req.method),
            }),
        }
    }

    async fn handle_projects_list(&self) -> Result<Value, RpcMethodError> {
        let sessions_dir = self.sessions_dir.clone();
        tokio::task::spawn_blocking(move || build_projects_list(&sessions_dir))
            .await
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })
    }

    async fn handle_sessions_list(
        &self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            project_id: String,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let project_id = params.project_id.trim().to_string();
        if project_id.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing project_id".to_string(),
            });
        }

        let sessions_dir = self.sessions_dir.clone();
        tokio::task::spawn_blocking(move || build_sessions_list(&sessions_dir, &project_id))
            .await
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })
    }

    async fn handle_sessions_get_timeline(
        &self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            session_id: String,
            limit: Option<u32>,
            cursor: Option<u64>,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let session_id = params.session_id.trim().to_string();
        if session_id.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing session_id".to_string(),
            });
        }

        let limit = params.limit.unwrap_or(200).clamp(1, 1000) as usize;
        let cursor = params.cursor.unwrap_or(0);

        let sessions_dir = self.sessions_dir.clone();
        tokio::task::spawn_blocking(move || {
            build_session_timeline(&sessions_dir, &session_id, limit, cursor)
        })
        .await
        .map_err(|error| RpcMethodError {
            code: "Error".to_string(),
            message: error.to_string(),
        })?
    }

    async fn handle_sessions_subscribe_timeline(
        &mut self,
        client_session_id: &str,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            session_id: String,
            from_cursor: Option<u64>,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let target_session_id = params.session_id.trim().to_string();
        if target_session_id.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing session_id".to_string(),
            });
        }

        let sessions_dir = self.sessions_dir.clone();
        let resolve_session_id = target_session_id.clone();
        let (log_path, file_size_bytes) = tokio::task::spawn_blocking(move || {
            resolve_session_log_path_and_size(&sessions_dir, &resolve_session_id)
        })
        .await
        .map_err(|error| RpcMethodError {
            code: "Error".to_string(),
            message: error.to_string(),
        })??;

        let cursor = params.from_cursor.unwrap_or(file_size_bytes);
        let subscription_id = Uuid::new_v4().to_string();
        self.timeline_subscriptions.insert(
            subscription_id.clone(),
            TimelineSubscription {
                session_id: client_session_id.to_string(),
                target_session_id: target_session_id.clone(),
                log_path,
                cursor_bytes: cursor,
                limit: 200,
                next_poll_at: Instant::now(),
                poll_every: Duration::from_millis(1_000),
            },
        );

        Ok(serde_json::json!({
            "subscription_id": subscription_id,
            "cursor": cursor,
        }))
    }

    async fn handle_tasks_list(&self) -> Result<Value, RpcMethodError> {
        tokio::task::spawn_blocking(build_tasks_list)
            .await
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })?
    }

    async fn handle_tasks_get(
        &self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            task_id: String,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let task_id = params.task_id.trim().to_string();
        if task_id.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing task_id".to_string(),
            });
        }

        tokio::task::spawn_blocking(move || build_tasks_get(&task_id))
            .await
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })?
    }

    async fn handle_tasks_create(
        &self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            project_path: String,
            body: String,
            images: Option<Vec<String>>,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let project_path = params.project_path.trim().to_string();
        if project_path.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing project_path".to_string(),
            });
        }
        let body = params.body.trim().to_string();
        if body.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing body".to_string(),
            });
        }

        let images = params.images.unwrap_or_default();
        tokio::task::spawn_blocking(move || build_tasks_create(&project_path, &body, &images))
            .await
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })?
    }

    async fn handle_tasks_delete(
        &self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            task_id: String,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let task_id = params.task_id.trim().to_string();
        if task_id.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing task_id".to_string(),
            });
        }

        tokio::task::spawn_blocking(move || build_tasks_delete(&task_id))
            .await
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })?
    }

    async fn handle_tasks_spawn(
        &mut self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            task_id: String,
            engine: String,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let task_id = params.task_id.trim().to_string();
        if task_id.is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing task_id".to_string(),
            });
        }

        let engine = parse_agent_engine(&params.engine)?;
        let (task, images) =
            tokio::task::spawn_blocking(move || build_tasks_load_for_spawn(&task_id))
                .await
                .map_err(|error| RpcMethodError {
                    code: "Error".to_string(),
                    message: error.to_string(),
                })??;

        let prompt = crate::domain::format_task_spawn_prompt(&task, &images);
        let spawned = self
            .process_manager
            .spawn_agent_process(engine, &task.project_path, &prompt, SpawnIoMode::Pipes)
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })?;

        let process_id = spawned.id.clone();
        self.processes.insert(
            process_id.clone(),
            ProcessEntry {
                process: spawned,
                status: ProcessStatus::Running,
                exit_code: None,
                session_id: None,
                session_log_path: None,
            },
        );

        Ok(serde_json::json!({ "process_id": process_id }))
    }

    fn handle_get_info(&self, opts: &super::ServeOptions, connection_guid: &str) -> Value {
        let mut capabilities = vec!["control-v1".to_string()];
        if opts.enable_shell {
            capabilities.push("shell-v1".to_string());
        }
        serde_json::json!({
            "ccbox_id": connection_guid,
            "label": opts.label.clone(),
            "version": env!("CARGO_PKG_VERSION"),
            "capabilities": capabilities,
            "now_ts": super::now_iso(),
        })
    }

    async fn handle_agents_spawn(
        &mut self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            engine: String,
            project_path: String,
            prompt: String,
            io_mode: Option<String>,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let engine = parse_agent_engine(&params.engine)?;
        let io_mode = parse_io_mode(params.io_mode.as_deref())?;

        let project_path = PathBuf::from(params.project_path.trim());
        if project_path.as_os_str().is_empty() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "missing project_path".to_string(),
            });
        }
        let meta = fs::metadata(&project_path).map_err(|error| RpcMethodError {
            code: "InvalidParams".to_string(),
            message: format!("invalid project_path: {error}"),
        })?;
        if !meta.is_dir() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "project_path is not a directory".to_string(),
            });
        }

        let prompt = params.prompt;
        let spawned = self
            .process_manager
            .spawn_agent_process(engine, &project_path, &prompt, io_mode)
            .map_err(|error| RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            })?;

        let process_id = spawned.id.clone();
        self.processes.insert(
            process_id.clone(),
            ProcessEntry {
                process: spawned,
                status: ProcessStatus::Running,
                exit_code: None,
                session_id: None,
                session_log_path: None,
            },
        );

        Ok(serde_json::json!({ "process_id": process_id }))
    }

    fn handle_processes_list(&self) -> Value {
        let processes = self
            .processes
            .values()
            .map(|entry| {
                let started_ts = system_time_to_iso(entry.process.started_at);
                serde_json::json!({
                    "process_id": entry.process.id.clone(),
                    "engine": agent_engine_label(entry.process.engine),
                    "status": match entry.status { ProcessStatus::Running => "running", ProcessStatus::Exited => "exited" },
                    "started_ts": started_ts,
                    "project_path": entry.process.project_path.display().to_string(),
                    "session_id": entry.session_id.clone(),
                    "exit_code": entry.exit_code,
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({ "processes": processes })
    }

    async fn handle_processes_kill(
        &mut self,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            process_id: String,
        }
        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        self.process_manager
            .kill(params.process_id.trim())
            .map_err(|error| match error {
                crate::infra::KillProcessError::NotFound => RpcMethodError {
                    code: "NotFound".to_string(),
                    message: "process not found".to_string(),
                },
                crate::infra::KillProcessError::Kill(err) => RpcMethodError {
                    code: "Error".to_string(),
                    message: err.to_string(),
                },
            })?;

        Ok(serde_json::json!({ "killed": true }))
    }

    async fn handle_processes_subscribe_logs(
        &mut self,
        session_id: &str,
        req: &super::RpcRequestPayload,
    ) -> Result<Value, RpcMethodError> {
        #[derive(Debug, Deserialize)]
        struct Params {
            process_id: String,
            stream: String,
            from_offset: Option<u64>,
        }

        let params: Params =
            serde_json::from_value(req.params.clone()).map_err(|_| RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "invalid params".to_string(),
            })?;

        let process_id = params.process_id.trim();
        let Some(entry) = self.processes.get(process_id) else {
            return Err(RpcMethodError {
                code: "NotFound".to_string(),
                message: "process not found".to_string(),
            });
        };

        let stream = parse_log_stream(&params.stream)?;
        if resolve_log_path(&entry.process.io, stream).is_none() {
            return Err(RpcMethodError {
                code: "InvalidParams".to_string(),
                message: "stream not available for this process".to_string(),
            });
        }

        let offset = params.from_offset.unwrap_or(0);
        let subscription_id = Uuid::new_v4().to_string();
        self.log_subscriptions.insert(
            subscription_id.clone(),
            LogSubscription {
                session_id: session_id.to_string(),
                process_id: process_id.to_string(),
                stream,
                offset,
            },
        );

        Ok(serde_json::json!({
            "subscription_id": subscription_id,
            "offset": offset,
        }))
    }

    fn drain_process_signals(&mut self) {
        while let Ok(signal) = self.process_signal_rx.try_recv() {
            match signal {
                ProcessSignal::SessionMeta {
                    process_id,
                    session_id,
                } => {
                    if let Some(entry) = self.processes.get_mut(&process_id) {
                        entry.session_id = Some(session_id);
                    }
                }
                ProcessSignal::SessionLogPath {
                    process_id,
                    log_path,
                } => {
                    if let Some(entry) = self.processes.get_mut(&process_id) {
                        entry.session_log_path = Some(log_path);
                    }
                }
            }
        }
    }

    fn drain_process_exits(&mut self) {
        for exit in self.process_manager.poll_exits() {
            if let Some(entry) = self.processes.get_mut(&exit.process_id) {
                entry.status = ProcessStatus::Exited;
                entry.exit_code = exit.exit_code;
            }
        }
    }

    async fn drain_log_subscriptions(
        &mut self,
        ws: &mut super::WsStream,
    ) -> Result<(), super::ServeError> {
        let ids = self.log_subscriptions.keys().cloned().collect::<Vec<_>>();

        for id in ids {
            let maybe_event = {
                let Some(sub) = self.log_subscriptions.get_mut(&id) else {
                    continue;
                };
                let Some(entry) = self.processes.get(&sub.process_id) else {
                    self.log_subscriptions.remove(&id);
                    continue;
                };
                let Some(path) = resolve_log_path(&entry.process.io, sub.stream) else {
                    continue;
                };

                let (chunk, next_offset) = match read_bytes_from_offset(path, sub.offset, 32 * 1024)
                {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if chunk.is_empty() {
                    continue;
                }
                sub.offset = next_offset;

                let chunk_b64 = base64::engine::general_purpose::STANDARD.encode(&chunk);
                let payload = ProcessesLogEvent {
                    process_id: sub.process_id.clone(),
                    stream: log_stream_label(sub.stream).to_string(),
                    offset: next_offset,
                    chunk_b64,
                };
                Some((sub.session_id.clone(), payload))
            };

            if let Some((session_id, payload)) = maybe_event {
                self.send_event(ws, &session_id, "processes.log", payload)
                    .await?;
            }
        }

        Ok(())
    }

    async fn drain_timeline_subscriptions(
        &mut self,
        ws: &mut super::WsStream,
    ) -> Result<(), super::ServeError> {
        let now = Instant::now();
        let due_ids = self
            .timeline_subscriptions
            .iter()
            .filter(|(_, sub)| now >= sub.next_poll_at)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();

        for id in due_ids {
            let Some((
                client_session_id,
                target_session_id,
                log_path,
                cursor_bytes,
                limit,
                poll_every,
            )) = self.timeline_subscriptions.get(&id).map(|sub| {
                (
                    sub.session_id.clone(),
                    sub.target_session_id.clone(),
                    sub.log_path.clone(),
                    sub.cursor_bytes,
                    sub.limit,
                    sub.poll_every,
                )
            })
            else {
                continue;
            };

            let update = tokio::task::spawn_blocking(move || {
                compute_timeline_update(&log_path, cursor_bytes, limit, target_session_id)
            })
            .await;

            let update = match update {
                Ok(Ok(value)) => value,
                Ok(Err(TimelineUpdateError::NotFound)) => {
                    self.timeline_subscriptions.remove(&id);
                    continue;
                }
                Ok(Err(_)) => None,
                Err(_) => None,
            };

            if let Some(sub) = self.timeline_subscriptions.get_mut(&id) {
                sub.next_poll_at = now + poll_every;
                if let Some(ref update) = update {
                    sub.cursor_bytes = update.cursor_bytes;
                }
            }

            if let Some(update) = update {
                self.send_event(ws, &client_session_id, "sessions.timeline", update.event)
                    .await?;
            }
        }

        Ok(())
    }

    async fn send_event<P: Serialize>(
        &self,
        ws: &mut super::WsStream,
        session_id: &str,
        topic: &'static str,
        data: P,
    ) -> Result<(), super::ServeError> {
        let event_env = super::EnvelopeOut {
            v: super::REMOTE_PROTOCOL_VERSION,
            type_: "event",
            ts: super::now_iso(),
            payload: EventPayload { topic, data },
        };

        let event_text = serde_json::to_string(&event_env)?;
        let out_frame = super::EnvelopeOut {
            v: super::REMOTE_PROTOCOL_VERSION,
            type_: "mux/frame",
            ts: super::now_iso(),
            payload: super::MuxFramePayloadOut {
                session_id: session_id.to_string(),
                stream_id: super::CONTROL_V1_STREAM_ID,
                payload_b64: base64::engine::general_purpose::STANDARD
                    .encode(event_text.as_bytes()),
            },
        };

        super::send_json(ws, &out_frame).await
    }
}

#[derive(Debug)]
struct RpcMethodError {
    code: String,
    message: String,
}

#[derive(Serialize)]
struct EventPayload<P: Serialize> {
    topic: &'static str,
    data: P,
}

#[derive(Serialize)]
struct ProcessesLogEvent {
    process_id: String,
    stream: String,
    offset: u64,
    chunk_b64: String,
}

struct TimelineUpdate {
    cursor_bytes: u64,
    event: SessionsTimelineEvent,
}

enum TimelineUpdateError {
    NotFound,
    Error,
}

#[derive(Serialize)]
struct SessionsTimelineEvent {
    session_id: String,
    cursor: u64,
    items: Vec<TimelineItemOut>,
}

#[derive(Serialize)]
struct TimelineItemOut {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_line_no: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp_ms: Option<i64>,
    summary: String,
    detail: String,
}

fn parse_agent_engine(raw: &str) -> Result<AgentEngine, RpcMethodError> {
    match raw.trim().to_lowercase().as_str() {
        "codex" => Ok(AgentEngine::Codex),
        "claude" => Ok(AgentEngine::Claude),
        _ => Err(RpcMethodError {
            code: "InvalidParams".to_string(),
            message: "invalid engine".to_string(),
        }),
    }
}

fn parse_io_mode(raw: Option<&str>) -> Result<SpawnIoMode, RpcMethodError> {
    match raw.map(|s| s.trim().to_lowercase()) {
        None => Ok(SpawnIoMode::Pipes),
        Some(v) if v == "pipes" => Ok(SpawnIoMode::Pipes),
        Some(v) if v == "tty" => Ok(SpawnIoMode::Tty),
        Some(_) => Err(RpcMethodError {
            code: "InvalidParams".to_string(),
            message: "invalid io_mode".to_string(),
        }),
    }
}

fn agent_engine_label(engine: AgentEngine) -> &'static str {
    match engine {
        AgentEngine::Codex => "codex",
        AgentEngine::Claude => "claude",
    }
}

fn parse_log_stream(raw: &str) -> Result<LogStream, RpcMethodError> {
    match raw.trim().to_lowercase().as_str() {
        "stdout" => Ok(LogStream::Stdout),
        "stderr" => Ok(LogStream::Stderr),
        "combined" => Ok(LogStream::Combined),
        _ => Err(RpcMethodError {
            code: "InvalidParams".to_string(),
            message: "invalid stream".to_string(),
        }),
    }
}

fn log_stream_label(stream: LogStream) -> &'static str {
    match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
        LogStream::Combined => "combined",
    }
}

fn resolve_log_path(io: &SpawnedAgentIo, stream: LogStream) -> Option<&Path> {
    match (io, stream) {
        (SpawnedAgentIo::Pipes { stdout_path, .. }, LogStream::Stdout) => Some(stdout_path),
        (SpawnedAgentIo::Pipes { stderr_path, .. }, LogStream::Stderr) => Some(stderr_path),
        (SpawnedAgentIo::Pipes { log_path, .. }, LogStream::Combined) => Some(log_path),
        (
            SpawnedAgentIo::Tty {
                transcript_path, ..
            },
            LogStream::Combined,
        ) => Some(transcript_path),
        _ => None,
    }
}

fn read_bytes_from_offset(
    path: &Path,
    offset: u64,
    max_bytes: usize,
) -> io::Result<(Vec<u8>, u64)> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    if offset >= size {
        return Ok((Vec::new(), offset));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; max_bytes.min((size - offset) as usize)];
    let n = file.read(&mut buf)?;
    buf.truncate(n);
    Ok((buf, offset + n as u64))
}

fn session_engine_label(engine: crate::domain::SessionEngine) -> &'static str {
    match engine {
        crate::domain::SessionEngine::Codex => "codex",
        crate::domain::SessionEngine::Claude => "claude",
        crate::domain::SessionEngine::Gemini => "gemini",
        crate::domain::SessionEngine::OpenCode => "opencode",
    }
}

fn build_sessions_list(sessions_dir: &Path, project_id: &str) -> Value {
    let scan = crate::infra::scan_all_sessions(sessions_dir);
    let mut sessions = scan
        .sessions
        .into_iter()
        .filter(|session| session.meta.cwd.display().to_string() == project_id)
        .collect::<Vec<_>>();

    sessions.sort_by_key(|session| session.file_modified.unwrap_or(SystemTime::UNIX_EPOCH));
    sessions.reverse();

    let rows = sessions
        .into_iter()
        .map(|session| SessionListEntry {
            session_id: session.meta.id,
            started_ts: session.meta.started_at_rfc3339,
            engine: session_engine_label(session.engine).to_string(),
            title: session.title,
        })
        .collect::<Vec<_>>();

    serde_json::to_value(SessionsListResult { sessions: rows })
        .unwrap_or_else(|_| serde_json::json!({ "sessions": [] }))
}

fn resolve_session_log_path_and_size(
    sessions_dir: &Path,
    session_id: &str,
) -> Result<(PathBuf, u64), RpcMethodError> {
    let scan = crate::infra::scan_all_sessions(sessions_dir);
    let Some(session) = scan.sessions.into_iter().find(|s| s.meta.id == session_id) else {
        return Err(RpcMethodError {
            code: "NotFound".to_string(),
            message: "session not found".to_string(),
        });
    };

    let size = match fs::metadata(&session.log_path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(RpcMethodError {
                code: "NotFound".to_string(),
                message: "session log not found".to_string(),
            });
        }
        Err(error) => {
            return Err(RpcMethodError {
                code: "Error".to_string(),
                message: error.to_string(),
            });
        }
    };

    Ok((session.log_path, size))
}

fn timeline_kind_label(kind: crate::domain::TimelineItemKind) -> &'static str {
    match kind {
        crate::domain::TimelineItemKind::Turn => "turn",
        crate::domain::TimelineItemKind::User => "user",
        crate::domain::TimelineItemKind::Assistant => "assistant",
        crate::domain::TimelineItemKind::Thinking => "thinking",
        crate::domain::TimelineItemKind::ToolCall => "tool_call",
        crate::domain::TimelineItemKind::ToolOutput => "tool_output",
        crate::domain::TimelineItemKind::TokenCount => "token_count",
        crate::domain::TimelineItemKind::Note => "note",
    }
}

fn timeline_item_to_out(item: crate::domain::TimelineItem) -> TimelineItemOut {
    TimelineItemOut {
        kind: timeline_kind_label(item.kind),
        turn_id: item.turn_id,
        call_id: item.call_id,
        source_line_no: item.source_line_no,
        timestamp: item.timestamp,
        timestamp_ms: item.timestamp_ms,
        summary: item.summary,
        detail: item.detail,
    }
}

fn build_session_timeline(
    sessions_dir: &Path,
    session_id: &str,
    limit: usize,
    cursor: u64,
) -> Result<Value, RpcMethodError> {
    let (log_path, file_size_bytes) = resolve_session_log_path_and_size(sessions_dir, session_id)?;
    if cursor >= file_size_bytes {
        return Ok(serde_json::json!({
            "items": [],
            "next_cursor": file_size_bytes,
        }));
    }

    let timeline =
        crate::infra::load_session_timeline(&log_path).map_err(|error| RpcMethodError {
            code: "Error".to_string(),
            message: error.to_string(),
        })?;

    let mut items = timeline.items;
    if items.len() > limit {
        items = items.split_off(items.len().saturating_sub(limit));
    }

    let out_items = items
        .into_iter()
        .map(timeline_item_to_out)
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "items": out_items,
        "next_cursor": file_size_bytes,
        "warnings": timeline.warnings,
        "truncated": timeline.truncated,
    }))
}

fn compute_timeline_update(
    log_path: &Path,
    cursor_bytes: u64,
    limit: usize,
    target_session_id: String,
) -> Result<Option<TimelineUpdate>, TimelineUpdateError> {
    let file_size_bytes = match fs::metadata(log_path) {
        Ok(meta) => meta.len(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(TimelineUpdateError::NotFound);
        }
        Err(_) => return Err(TimelineUpdateError::Error),
    };

    if file_size_bytes <= cursor_bytes {
        return Ok(None);
    }

    let timeline =
        crate::infra::load_session_timeline(log_path).map_err(|_| TimelineUpdateError::Error)?;

    let mut items = timeline.items;
    if items.len() > limit {
        items = items.split_off(items.len().saturating_sub(limit));
    }

    let out_items = items
        .into_iter()
        .map(timeline_item_to_out)
        .collect::<Vec<_>>();

    Ok(Some(TimelineUpdate {
        cursor_bytes: file_size_bytes,
        event: SessionsTimelineEvent {
            session_id: target_session_id,
            cursor: file_size_bytes,
            items: out_items,
        },
    }))
}

fn build_tasks_list() -> Result<Value, RpcMethodError> {
    let store = crate::infra::TaskStore::open_default().map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;
    let tasks = store.list_tasks().map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;

    let rows = tasks
        .into_iter()
        .map(|entry| {
            serde_json::json!({
                "task_id": entry.task.id.to_string(),
                "title": crate::domain::derive_task_title(&entry.task.body),
                "project_path": entry.task.project_path.display().to_string(),
                "updated_ts": system_time_to_iso(entry.task.updated_at).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({ "tasks": rows }))
}

fn build_tasks_get(task_id: &str) -> Result<Value, RpcMethodError> {
    let store = crate::infra::TaskStore::open_default().map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;

    let id = crate::domain::TaskId::new(task_id.to_string());
    let loaded = store.load_task(&id).map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;
    let Some((task, images)) = loaded else {
        return Err(RpcMethodError {
            code: "NotFound".to_string(),
            message: "task not found".to_string(),
        });
    };

    let images_out = images
        .into_iter()
        .map(|image| {
            serde_json::json!({
                "ordinal": image.ordinal,
                "source_path": image.source_path.display().to_string(),
            })
        })
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "task": {
            "task_id": task.id.to_string(),
            "project_path": task.project_path.display().to_string(),
            "body": task.body,
            "created_ts": system_time_to_iso(task.created_at).unwrap_or_default(),
            "updated_ts": system_time_to_iso(task.updated_at).unwrap_or_default(),
            "images": images_out,
        }
    }))
}

fn build_tasks_create(
    project_path: &str,
    body: &str,
    images: &[String],
) -> Result<Value, RpcMethodError> {
    let store = crate::infra::TaskStore::open_default().map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;

    let project_path = PathBuf::from(project_path);
    let image_paths = images
        .iter()
        .map(|p| PathBuf::from(p.trim()))
        .filter(|p| !p.as_os_str().is_empty())
        .collect::<Vec<_>>();

    let id = store
        .create_task(&project_path, body, &image_paths)
        .map_err(|error| RpcMethodError {
            code: "Error".to_string(),
            message: error.to_string(),
        })?;

    Ok(serde_json::json!({ "task_id": id.to_string() }))
}

fn build_tasks_delete(task_id: &str) -> Result<Value, RpcMethodError> {
    let store = crate::infra::TaskStore::open_default().map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;

    let id = crate::domain::TaskId::new(task_id.to_string());
    let deleted = store.delete_task(&id).map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;

    Ok(serde_json::json!({ "deleted": deleted }))
}

fn build_tasks_load_for_spawn(
    task_id: &str,
) -> Result<(crate::domain::Task, Vec<crate::domain::TaskImage>), RpcMethodError> {
    let store = crate::infra::TaskStore::open_default().map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;

    let id = crate::domain::TaskId::new(task_id.to_string());
    let loaded = store.load_task(&id).map_err(|error| RpcMethodError {
        code: "Error".to_string(),
        message: error.to_string(),
    })?;
    let Some((task, images)) = loaded else {
        return Err(RpcMethodError {
            code: "NotFound".to_string(),
            message: "task not found".to_string(),
        });
    };

    Ok((task, images))
}

fn build_projects_list(sessions_dir: &Path) -> Value {
    let scan = crate::infra::scan_all_sessions(sessions_dir);
    let projects = crate::domain::index_projects(&scan.sessions);
    let rows = projects
        .into_iter()
        .map(|project| ProjectListEntry {
            project_id: project.project_path.display().to_string(),
            path: project.project_path.display().to_string(),
            last_seen_ts: project.last_modified.and_then(system_time_to_iso),
            session_count: project.sessions.len() as u32,
        })
        .collect::<Vec<_>>();
    serde_json::to_value(ProjectsListResult { projects: rows })
        .unwrap_or_else(|_| serde_json::json!({ "projects": [] }))
}

#[derive(Debug, Serialize)]
struct ProjectListEntry {
    project_id: String,
    path: String,
    last_seen_ts: Option<String>,
    session_count: u32,
}

#[derive(Debug, Serialize)]
struct ProjectsListResult {
    projects: Vec<ProjectListEntry>,
}

#[derive(Debug, Serialize)]
struct SessionListEntry {
    session_id: String,
    started_ts: String,
    engine: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct SessionsListResult {
    sessions: Vec<SessionListEntry>,
}

fn system_time_to_iso(value: SystemTime) -> Option<String> {
    let timestamp = OffsetDateTime::from(value);
    timestamp.format(&Rfc3339).ok()
}
