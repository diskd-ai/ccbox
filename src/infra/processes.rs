use crate::domain::AgentEngine;
use crate::domain::SpawnIoMode;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use time::{Duration as TimeDuration, OffsetDateTime};

const SESSION_LOG_WAIT_TIMEOUT: Duration = Duration::from_secs(20);
const SESSION_LOG_WAIT_POLL: Duration = Duration::from_millis(200);

#[derive(Clone, Debug)]
pub enum SpawnedAgentIo {
    Pipes {
        stdout_path: PathBuf,
        stderr_path: PathBuf,
        log_path: PathBuf,
    },
    Tty {
        transcript_path: PathBuf,
        log_path: PathBuf,
    },
}

#[derive(Clone, Debug)]
pub struct SpawnedAgentProcess {
    pub id: String,
    pub pid: u32,
    pub engine: AgentEngine,
    pub project_path: PathBuf,
    pub started_at: SystemTime,
    pub prompt_preview: String,
    pub io: SpawnedAgentIo,
}

#[derive(Clone, Debug)]
pub enum ProcessSignal {
    SessionMeta {
        process_id: String,
        session_id: String,
    },
    SessionLogPath {
        process_id: String,
        log_path: PathBuf,
    },
}

#[derive(Clone, Debug)]
pub struct ProcessExit {
    pub process_id: String,
    pub exit_code: Option<i32>,
}

#[derive(Debug, Error)]
pub enum ProcessManagerError {
    #[error("failed to create process logs directory: {0}")]
    CreateLogsDir(#[from] io::Error),
}

#[derive(Debug, Error)]
pub enum SpawnAgentProcessError {
    #[error("failed to create process directory: {0}")]
    CreateProcessDir(io::Error),

    #[error("failed to write prompt file: {0}")]
    WritePrompt(io::Error),

    #[error("failed to spawn process: {0}")]
    Spawn(io::Error),

    #[error("failed to create PTY: {0}")]
    OpenPty(String),

    #[error("failed to spawn PTY process: {0}")]
    SpawnPty(String),

    #[error("failed to open PTY reader: {0}")]
    OpenPtyReader(String),

    #[error("failed to open PTY writer: {0}")]
    OpenPtyWriter(String),

    #[error("failed to open stdout log: {0}")]
    OpenStdout(io::Error),

    #[error("failed to open stderr log: {0}")]
    OpenStderr(io::Error),

    #[error("failed to open combined log: {0}")]
    OpenLog(io::Error),
}

#[derive(Debug, Error)]
pub enum KillProcessError {
    #[error("process not found")]
    NotFound,

    #[error("failed to kill process: {0}")]
    Kill(io::Error),
}

#[derive(Debug, Error)]
pub enum AttachTtyError {
    #[error("process not found")]
    NotFound,

    #[error("process is not a TTY session")]
    NotTty,

    #[error("failed to attach: {0}")]
    Attach(String),
}

#[derive(Debug, Error)]
pub enum WriteTtyError {
    #[error("process not found")]
    NotFound,

    #[error("process is not a TTY session")]
    NotTty,

    #[error("failed to write to TTY: {0}")]
    Write(io::Error),
}

#[derive(Debug, Error)]
pub enum ResizeTtyError {
    #[error("process not found")]
    NotFound,

    #[error("process is not a TTY session")]
    NotTty,

    #[error("failed to resize TTY: {0}")]
    Resize(String),
}

pub struct ProcessManager {
    sessions_dir: PathBuf,
    logs_dir: PathBuf,
    tx: Sender<ProcessSignal>,
    next_id: u64,
    pipes_children: HashMap<String, Child>,
    tty_children: HashMap<String, TtyProcess>,
}

struct TtyProcess {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    live_tx: Arc<Mutex<Option<Sender<Vec<u8>>>>>,
}

impl ProcessManager {
    pub fn new(
        sessions_dir: PathBuf,
        tx: Sender<ProcessSignal>,
    ) -> Result<Self, ProcessManagerError> {
        let logs_dir = sessions_dir.join(".ccbox").join("processes");
        fs::create_dir_all(&logs_dir)?;

        Ok(Self {
            sessions_dir,
            logs_dir,
            tx,
            next_id: 1,
            pipes_children: HashMap::new(),
            tty_children: HashMap::new(),
        })
    }

    pub fn spawn_agent_process(
        &mut self,
        engine: AgentEngine,
        project_path: &Path,
        prompt: &str,
        io_mode: SpawnIoMode,
    ) -> Result<SpawnedAgentProcess, SpawnAgentProcessError> {
        match io_mode {
            SpawnIoMode::Pipes => self.spawn_agent_process_pipes(engine, project_path, prompt),
            SpawnIoMode::Tty => self.spawn_agent_process_tty(engine, project_path, prompt),
        }
    }

    pub fn spawn_codex_resume_process(
        &mut self,
        project_path: &Path,
        session_id: &str,
        prompt: &str,
    ) -> Result<SpawnedAgentProcess, SpawnAgentProcessError> {
        let started_at = SystemTime::now();
        let id = format!("p{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);

        let process_dir = self.logs_dir.join(&id);
        fs::create_dir_all(&process_dir).map_err(SpawnAgentProcessError::CreateProcessDir)?;

        let prompt_path = process_dir.join("prompt.txt");
        fs::write(&prompt_path, prompt).map_err(SpawnAgentProcessError::WritePrompt)?;

        let stdout_path = process_dir.join("stdout.log");
        let stderr_path = process_dir.join("stderr.log");
        let log_path = process_dir.join("process.log");

        let stdout_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&stdout_path)
            .map_err(SpawnAgentProcessError::OpenStdout)?;
        let stderr_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&stderr_path)
            .map_err(SpawnAgentProcessError::OpenStderr)?;
        let combined_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&log_path)
            .map_err(SpawnAgentProcessError::OpenLog)?;

        let combined_writer = Arc::new(Mutex::new(io::BufWriter::new(combined_file)));
        {
            let mut writer = combined_writer.lock().map_err(|_| {
                SpawnAgentProcessError::OpenLog(io::Error::other("log lock poisoned"))
            })?;
            let _ = writeln!(
                writer,
                "engine: {}\nmode: resume\nresume_session_id: {}\nproject: {}\nstarted_at: {:?}\n---",
                AgentEngine::Codex.label(),
                session_id,
                project_path.display(),
                started_at
            );
        }

        let prompt_preview = first_non_empty_line(prompt)
            .unwrap_or_else(|| "(empty prompt)".to_string())
            .chars()
            .take(120)
            .collect::<String>();

        let mut command =
            build_codex_exec_resume_command(project_path, session_id, &self.sessions_dir);
        let mut child = command.spawn().map_err(SpawnAgentProcessError::Spawn)?;
        let pid = child.id();

        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(prompt.as_bytes());
            let _ = stdin.write_all(b"\n");
        }

        let stdout = child.stdout.take().ok_or_else(|| {
            SpawnAgentProcessError::OpenStdout(io::Error::other("stdout missing"))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            SpawnAgentProcessError::OpenStderr(io::Error::other("stderr missing"))
        })?;

        let sessions_dir = self.sessions_dir.clone();
        let tx = self.tx.clone();
        let id_for_stdout = id.clone();
        let stdout_combined = combined_writer.clone();
        std::thread::spawn(move || {
            pipe_reader_thread(
                stdout,
                stdout_file,
                PipeReaderContext {
                    kind: StreamKind::Stdout,
                    combined: stdout_combined,
                    engine: AgentEngine::Codex,
                    sessions_dir,
                    process_id: id_for_stdout,
                    tx,
                },
            );
        });

        let stderr_combined = combined_writer.clone();
        std::thread::spawn(move || {
            pipe_reader_thread_simple(StreamKind::Stderr, stderr, stderr_file, stderr_combined);
        });

        self.pipes_children.insert(id.clone(), child);

        Ok(SpawnedAgentProcess {
            id,
            pid,
            engine: AgentEngine::Codex,
            project_path: project_path.to_path_buf(),
            started_at,
            prompt_preview,
            io: SpawnedAgentIo::Pipes {
                stdout_path,
                stderr_path,
                log_path,
            },
        })
    }

    fn spawn_agent_process_pipes(
        &mut self,
        engine: AgentEngine,
        project_path: &Path,
        prompt: &str,
    ) -> Result<SpawnedAgentProcess, SpawnAgentProcessError> {
        let started_at = SystemTime::now();
        let id = format!("p{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);

        let process_dir = self.logs_dir.join(&id);
        fs::create_dir_all(&process_dir).map_err(SpawnAgentProcessError::CreateProcessDir)?;

        let prompt_path = process_dir.join("prompt.txt");
        fs::write(&prompt_path, prompt).map_err(SpawnAgentProcessError::WritePrompt)?;

        let stdout_path = process_dir.join("stdout.log");
        let stderr_path = process_dir.join("stderr.log");
        let log_path = process_dir.join("process.log");

        let last_message_path = match engine {
            AgentEngine::Codex => Some(process_dir.join("last_message.txt")),
            AgentEngine::Claude => None,
        };

        let stdout_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&stdout_path)
            .map_err(SpawnAgentProcessError::OpenStdout)?;
        let stderr_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&stderr_path)
            .map_err(SpawnAgentProcessError::OpenStderr)?;
        let combined_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&log_path)
            .map_err(SpawnAgentProcessError::OpenLog)?;

        let combined_writer = Arc::new(Mutex::new(io::BufWriter::new(combined_file)));
        {
            let mut writer = combined_writer.lock().map_err(|_| {
                SpawnAgentProcessError::OpenLog(io::Error::other("log lock poisoned"))
            })?;
            let _ = writeln!(
                writer,
                "engine: {}\nproject: {}\nstarted_at: {:?}\n---",
                engine.label(),
                project_path.display(),
                started_at
            );
        }

        let prompt_preview = first_non_empty_line(prompt)
            .unwrap_or_else(|| "(empty prompt)".to_string())
            .chars()
            .take(120)
            .collect::<String>();

        let mut command = build_engine_command(
            engine,
            project_path,
            prompt,
            last_message_path.as_deref(),
            &self.sessions_dir,
        );
        let mut child = command.spawn().map_err(SpawnAgentProcessError::Spawn)?;
        let pid = child.id();

        if let Some(mut stdin) = child.stdin.take() {
            if matches!(engine, AgentEngine::Codex) {
                let _ = stdin.write_all(prompt.as_bytes());
                let _ = stdin.write_all(b"\n");
            }
        }

        let stdout = child.stdout.take().ok_or_else(|| {
            SpawnAgentProcessError::OpenStdout(io::Error::other("stdout missing"))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            SpawnAgentProcessError::OpenStderr(io::Error::other("stderr missing"))
        })?;

        let sessions_dir = self.sessions_dir.clone();
        let tx = self.tx.clone();
        let id_for_stdout = id.clone();
        let stdout_combined = combined_writer.clone();
        std::thread::spawn(move || {
            pipe_reader_thread(
                stdout,
                stdout_file,
                PipeReaderContext {
                    kind: StreamKind::Stdout,
                    combined: stdout_combined,
                    engine,
                    sessions_dir,
                    process_id: id_for_stdout,
                    tx,
                },
            );
        });

        let stderr_combined = combined_writer.clone();
        std::thread::spawn(move || {
            pipe_reader_thread_simple(StreamKind::Stderr, stderr, stderr_file, stderr_combined);
        });

        self.pipes_children.insert(id.clone(), child);

        Ok(SpawnedAgentProcess {
            id,
            pid,
            engine,
            project_path: project_path.to_path_buf(),
            started_at,
            prompt_preview,
            io: SpawnedAgentIo::Pipes {
                stdout_path,
                stderr_path,
                log_path,
            },
        })
    }

    fn spawn_agent_process_tty(
        &mut self,
        engine: AgentEngine,
        project_path: &Path,
        prompt: &str,
    ) -> Result<SpawnedAgentProcess, SpawnAgentProcessError> {
        let started_at = SystemTime::now();
        let id = format!("p{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);

        let process_dir = self.logs_dir.join(&id);
        fs::create_dir_all(&process_dir).map_err(SpawnAgentProcessError::CreateProcessDir)?;

        let prompt_path = process_dir.join("prompt.txt");
        fs::write(&prompt_path, prompt).map_err(SpawnAgentProcessError::WritePrompt)?;

        let log_path = process_dir.join("process.log");

        let prompt_preview = first_non_empty_line(prompt)
            .unwrap_or_else(|| "(empty prompt)".to_string())
            .chars()
            .take(120)
            .collect::<String>();

        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| SpawnAgentProcessError::OpenPty(error.to_string()))?;

        let command = build_engine_command_tty(engine, project_path, prompt, &self.sessions_dir);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| SpawnAgentProcessError::SpawnPty(error.to_string()))?;

        let pid = child.process_id().unwrap_or(0);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| SpawnAgentProcessError::OpenPtyReader(error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| SpawnAgentProcessError::OpenPtyWriter(error.to_string()))?;

        let live_tx = Arc::new(Mutex::new(None));
        let log_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&log_path)
            .map_err(SpawnAgentProcessError::OpenLog)?;

        let sessions_dir = self.sessions_dir.clone();
        let tx = self.tx.clone();
        let process_id = id.clone();
        let live_tx_thread = live_tx.clone();
        let project_path = project_path.to_path_buf();
        let project_path_for_thread = project_path.clone();
        std::thread::spawn(move || {
            pty_reader_thread(
                reader,
                log_file,
                PtyReaderContext {
                    engine,
                    sessions_dir,
                    process_id,
                    tx,
                    live_tx: live_tx_thread,
                    project_path: project_path_for_thread,
                    started_at,
                },
            );
        });

        self.tty_children.insert(
            id.clone(),
            TtyProcess {
                child,
                master: pair.master,
                writer,
                live_tx,
            },
        );

        Ok(SpawnedAgentProcess {
            id,
            pid,
            engine,
            project_path,
            started_at,
            prompt_preview,
            io: SpawnedAgentIo::Tty {
                transcript_path: log_path.clone(),
                log_path,
            },
        })
    }

    pub fn attach_tty_output(
        &mut self,
        process_id: &str,
    ) -> Result<std::sync::mpsc::Receiver<Vec<u8>>, AttachTtyError> {
        if self.pipes_children.contains_key(process_id) {
            return Err(AttachTtyError::NotTty);
        }
        let Some(process) = self.tty_children.get_mut(process_id) else {
            return Err(AttachTtyError::NotFound);
        };

        let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
        match process.live_tx.lock() {
            Ok(mut guard) => {
                *guard = Some(tx);
            }
            Err(_) => return Err(AttachTtyError::Attach("lock poisoned".to_string())),
        }
        Ok(rx)
    }

    pub fn detach_tty_output(&mut self, process_id: &str) {
        let Some(process) = self.tty_children.get_mut(process_id) else {
            return;
        };
        if let Ok(mut guard) = process.live_tx.lock() {
            *guard = None;
        }
    }

    pub fn write_tty(&mut self, process_id: &str, bytes: &[u8]) -> Result<(), WriteTtyError> {
        if self.pipes_children.contains_key(process_id) {
            return Err(WriteTtyError::NotTty);
        }
        let Some(process) = self.tty_children.get_mut(process_id) else {
            return Err(WriteTtyError::NotFound);
        };
        process
            .writer
            .write_all(bytes)
            .and_then(|()| process.writer.flush())
            .map_err(WriteTtyError::Write)
    }

    pub fn resize_tty(
        &mut self,
        process_id: &str,
        rows: u16,
        cols: u16,
    ) -> Result<(), ResizeTtyError> {
        if self.pipes_children.contains_key(process_id) {
            return Err(ResizeTtyError::NotTty);
        }
        let Some(process) = self.tty_children.get_mut(process_id) else {
            return Err(ResizeTtyError::NotFound);
        };
        process
            .master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| ResizeTtyError::Resize(error.to_string()))
    }

    pub fn poll_exits(&mut self) -> Vec<ProcessExit> {
        let mut exits = Vec::new();
        let mut finished = Vec::new();

        for (id, child) in &mut self.pipes_children {
            if let Ok(Some(status)) = child.try_wait() {
                finished.push(id.clone());
                exits.push(ProcessExit {
                    process_id: id.clone(),
                    exit_code: status.code(),
                });
            }
        }

        for id in finished {
            self.pipes_children.remove(&id);
        }

        let mut finished_tty = Vec::new();
        for (id, process) in &mut self.tty_children {
            if let Ok(Some(status)) = process.child.try_wait() {
                finished_tty.push(id.clone());
                let exit_code = i32::try_from(status.exit_code()).ok();
                exits.push(ProcessExit {
                    process_id: id.clone(),
                    exit_code,
                });
            }
        }

        for id in finished_tty {
            self.tty_children.remove(&id);
        }

        exits
    }

    pub fn kill(&mut self, process_id: &str) -> Result<(), KillProcessError> {
        if let Some(child) = self.pipes_children.get_mut(process_id) {
            child.kill().map_err(KillProcessError::Kill)?;
            return Ok(());
        }

        let Some(process) = self.tty_children.get_mut(process_id) else {
            return Err(KillProcessError::NotFound);
        };

        process.child.kill().map_err(KillProcessError::Kill)?;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamKind {
    Stdout,
    Stderr,
}

struct PipeReaderContext {
    kind: StreamKind,
    combined: Arc<Mutex<io::BufWriter<File>>>,
    engine: AgentEngine,
    sessions_dir: PathBuf,
    process_id: String,
    tx: Sender<ProcessSignal>,
}

struct PtyReaderContext {
    engine: AgentEngine,
    sessions_dir: PathBuf,
    process_id: String,
    tx: Sender<ProcessSignal>,
    live_tx: Arc<Mutex<Option<Sender<Vec<u8>>>>>,
    project_path: PathBuf,
    started_at: SystemTime,
}

fn build_engine_command(
    engine: AgentEngine,
    project_path: &Path,
    prompt: &str,
    last_message_path: Option<&Path>,
    sessions_dir: &Path,
) -> Command {
    match engine {
        AgentEngine::Codex => {
            let mut command = Command::new("codex");
            command.arg("exec").arg("--full-auto").arg("--json");
            if let Some(path) = last_message_path {
                command.arg("--output-last-message").arg(path);
            }
            command
                .arg("-C")
                .arg(project_path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(project_path)
                .env("CODEX_SESSIONS_DIR", sessions_dir)
                .arg("-");
            command
        }
        AgentEngine::Claude => {
            let mut command = Command::new("claude");
            command
                .arg("--dangerously-skip-permissions")
                .arg("--verbose")
                .arg("--output-format")
                .arg("stream-json")
                .arg("-p")
                .arg(prompt)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .current_dir(project_path);
            command
        }
    }
}

fn build_codex_exec_resume_command(
    project_path: &Path,
    session_id: &str,
    sessions_dir: &Path,
) -> Command {
    let mut command = Command::new("codex");
    command
        .arg("exec")
        .arg("resume")
        .arg("--full-auto")
        .arg("--json")
        .arg("-C")
        .arg(project_path)
        .arg(session_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(project_path)
        .env("CODEX_SESSIONS_DIR", sessions_dir)
        .arg("-");
    command
}

fn build_engine_command_tty(
    engine: AgentEngine,
    project_path: &Path,
    prompt: &str,
    sessions_dir: &Path,
) -> CommandBuilder {
    match engine {
        AgentEngine::Codex => {
            let mut command = CommandBuilder::new("codex");
            command.arg("--full-auto");
            command.arg("-C");
            command.arg(project_path);
            if !prompt.trim().is_empty() {
                command.arg(prompt);
            }
            command.cwd(project_path.as_os_str());
            command.env("CODEX_SESSIONS_DIR", sessions_dir);
            command
        }
        AgentEngine::Claude => {
            let mut command = CommandBuilder::new("claude");
            command.arg("--dangerously-skip-permissions");
            command.arg("--verbose");
            if !prompt.trim().is_empty() {
                command.arg(prompt);
            }
            command.cwd(project_path.as_os_str());
            command
        }
    }
}

fn pipe_reader_thread(pipe: impl Read, mut file: File, ctx: PipeReaderContext) {
    let mut reader = BufReader::new(pipe);
    let mut line = String::new();

    let mut sent_session_meta = false;

    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
                write_combined_line(&ctx.combined, ctx.kind, &line);

                if !sent_session_meta && ctx.engine == AgentEngine::Codex {
                    if let Some((session_id, started_at)) = parse_session_meta_line(&line) {
                        sent_session_meta = true;
                        let _ = ctx.tx.send(ProcessSignal::SessionMeta {
                            process_id: ctx.process_id.clone(),
                            session_id: session_id.clone(),
                        });
                        if let Some(log_path) = wait_for_session_log(
                            &ctx.sessions_dir,
                            &started_at,
                            &session_id,
                            SESSION_LOG_WAIT_TIMEOUT,
                        ) {
                            let _ = ctx.tx.send(ProcessSignal::SessionLogPath {
                                process_id: ctx.process_id.clone(),
                                log_path,
                            });
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
}

fn pty_reader_thread(mut reader: Box<dyn Read + Send>, file: File, ctx: PtyReaderContext) {
    let mut writer = io::BufWriter::new(file);
    let _ = writeln!(
        writer,
        "engine: {}\nproject: {}\nstarted_at: {:?}\n---",
        ctx.engine.label(),
        ctx.project_path.display(),
        ctx.started_at
    );
    let _ = writer.flush();

    let mut sent_session_meta = false;
    let mut buf = vec![0u8; 16_384];
    let mut line_buf: Vec<u8> = Vec::new();

    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        let chunk = &buf[..n];
        let _ = writer.write_all(chunk);
        let _ = writer.flush();

        if let Ok(mut guard) = ctx.live_tx.lock() {
            if let Some(tx) = guard.as_ref() {
                if tx.send(chunk.to_vec()).is_err() {
                    *guard = None;
                }
            }
        }

        if !sent_session_meta && ctx.engine == AgentEngine::Codex {
            line_buf.extend_from_slice(chunk);

            while let Some(pos) = line_buf.iter().position(|byte| *byte == b'\n') {
                let line = line_buf.drain(..=pos).collect::<Vec<u8>>();
                let Ok(line) = std::str::from_utf8(&line) else {
                    continue;
                };
                if let Some((session_id, started_at)) = parse_session_meta_line(line.trim_end()) {
                    sent_session_meta = true;
                    let _ = ctx.tx.send(ProcessSignal::SessionMeta {
                        process_id: ctx.process_id.clone(),
                        session_id: session_id.clone(),
                    });
                    if let Some(log_path) = wait_for_session_log(
                        &ctx.sessions_dir,
                        &started_at,
                        &session_id,
                        SESSION_LOG_WAIT_TIMEOUT,
                    ) {
                        let _ = ctx.tx.send(ProcessSignal::SessionLogPath {
                            process_id: ctx.process_id.clone(),
                            log_path,
                        });
                    }
                }
            }

            if line_buf.len() > 512 * 1024 {
                line_buf.clear();
            }
        }
    }

    if let Ok(mut guard) = ctx.live_tx.lock() {
        *guard = None;
    }
}

fn pipe_reader_thread_simple(
    kind: StreamKind,
    pipe: impl Read,
    mut file: File,
    combined: Arc<Mutex<io::BufWriter<File>>>,
) {
    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let _ = file.write_all(line.as_bytes());
                let _ = file.flush();
                write_combined_line(&combined, kind, &line);
            }
            Err(_) => break,
        }
    }
}

fn write_combined_line(combined: &Arc<Mutex<io::BufWriter<File>>>, kind: StreamKind, line: &str) {
    let prefix = match kind {
        StreamKind::Stdout => "[stdout] ",
        StreamKind::Stderr => "[stderr] ",
    };

    if let Ok(mut writer) = combined.lock() {
        let _ = writer.write_all(prefix.as_bytes());
        let _ = writer.write_all(line.as_bytes());
        let _ = writer.flush();
    }
}

fn parse_session_meta_line(line: &str) -> Option<(String, String)> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return None;
    };
    if value.get("type").and_then(|v| v.as_str()) != Some("session_meta") {
        return None;
    }
    let payload = value.get("payload")?;
    let session_id = payload.get("id").and_then(|v| v.as_str())?;
    let started_at = payload.get("timestamp").and_then(|v| v.as_str())?;
    Some((session_id.to_string(), started_at.to_string()))
}

fn wait_for_session_log(
    sessions_dir: &Path,
    started_at_rfc3339: &str,
    session_id: &str,
    timeout: Duration,
) -> Option<PathBuf> {
    let deadline = SystemTime::now().checked_add(timeout)?;
    loop {
        if let Some(path) = find_session_log_path(sessions_dir, started_at_rfc3339, session_id) {
            return Some(path);
        }
        if SystemTime::now() >= deadline {
            return None;
        }
        std::thread::sleep(SESSION_LOG_WAIT_POLL);
    }
}

fn find_session_log_path(
    sessions_dir: &Path,
    started_at_rfc3339: &str,
    session_id: &str,
) -> Option<PathBuf> {
    let timestamp = OffsetDateTime::parse(started_at_rfc3339, &Rfc3339).ok()?;
    // Codex stores sessions under local date directories (YYYY/MM/DD), while `session_meta`
    // timestamps are emitted in UTC. Search the UTC day plus adjacent days to handle offsets.
    let candidates = [
        timestamp,
        timestamp.saturating_add(TimeDuration::days(1)),
        timestamp.saturating_sub(TimeDuration::days(1)),
    ];

    for candidate in candidates {
        if let Some(path) = find_session_log_in_day_dir(sessions_dir, candidate, session_id) {
            return Some(path);
        }
    }

    None
}

fn find_session_log_in_day_dir(
    sessions_dir: &Path,
    timestamp: OffsetDateTime,
    session_id: &str,
) -> Option<PathBuf> {
    let year = timestamp.year();
    let month = timestamp.month() as u8;
    let day = timestamp.day();

    let day_dir = sessions_dir
        .join(format!("{year:04}"))
        .join(format!("{month:02}"))
        .join(format!("{day:02}"));

    let entries = fs::read_dir(day_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if file_name.contains(session_id) {
            return Some(path);
        }
    }

    None
}

fn first_non_empty_line(text: &str) -> Option<String> {
    text.lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(|line| line.to_string())
}

pub fn read_tail(path: &Path, max_bytes: usize) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok((String::from_utf8_lossy(&buf).to_string(), size))
}

pub fn read_from_offset(path: &Path, offset: u64, max_bytes: usize) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if offset >= size {
        return Ok((String::new(), offset));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; max_bytes.min((size - offset) as usize)];
    let n = file.read(&mut buf)?;
    buf.truncate(n);
    Ok((String::from_utf8_lossy(&buf).to_string(), offset + n as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn finds_session_log_in_adjacent_day_dir_when_meta_is_utc() {
        let temp = tempdir().expect("tempdir");
        let sessions_dir = temp.path();
        let session_id = "019c20ca-aacc-7351-a288-442d5b380489";

        let expected = sessions_dir
            .join("2026")
            .join("02")
            .join("03")
            .join(format!("rollout-2026-02-03T00-57-58-{session_id}.jsonl"));
        fs::create_dir_all(expected.parent().expect("parent")).expect("mkdirs");
        fs::write(&expected, "").expect("write");

        let found = find_session_log_path(sessions_dir, "2026-02-02T23:57:58.860Z", session_id);

        assert_eq!(found, Some(expected));
    }

    #[test]
    fn finds_session_log_in_previous_day_dir_when_meta_date_is_ahead() {
        let temp = tempdir().expect("tempdir");
        let sessions_dir = temp.path();
        let session_id = "deadbeef-dead-beef-dead-beefdeadbeef";

        let expected = sessions_dir
            .join("2026")
            .join("02")
            .join("18")
            .join(format!("rollout-2026-02-18T23-58-00-{session_id}.jsonl"));
        fs::create_dir_all(expected.parent().expect("parent")).expect("mkdirs");
        fs::write(&expected, "").expect("write");

        let found = find_session_log_path(sessions_dir, "2026-02-19T01:58:00.000Z", session_id);

        assert_eq!(found, Some(expected));
    }
}
