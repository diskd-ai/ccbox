use crate::domain::{Task, TaskId, TaskImage};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sqlx::Connection as _;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use uuid::Uuid;

static TASKS_MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Error)]
pub enum TaskStoreError {
    #[error(transparent)]
    ResolveStateDir(#[from] super::ResolveCcboxStateDirError),

    #[error("failed to create tasks state dir {path}: {source}")]
    CreateStateDir { path: String, source: io::Error },

    #[error("failed to run tasks migrations: {0}")]
    Migrate(String),

    #[error("failed to open tasks DB at {path}: {source}")]
    OpenDb {
        path: String,
        source: rusqlite::Error,
    },

    #[error("failed to query tasks DB: {0}")]
    Query(#[from] rusqlite::Error),
}

#[derive(Clone, Debug)]
pub struct TaskListEntry {
    pub task: Task,
    pub image_count: u32,
}

#[derive(Clone, Debug)]
pub struct TaskStore {
    db_path: PathBuf,
}

impl TaskStore {
    pub fn open_default() -> Result<Self, TaskStoreError> {
        let db_path = resolve_tasks_db_path()?;
        Self::open(db_path)
    }

    pub fn open(db_path: PathBuf) -> Result<Self, TaskStoreError> {
        ensure_tasks_db_ready(&db_path)?;
        Ok(Self { db_path })
    }

    pub fn list_tasks(&self) -> Result<Vec<TaskListEntry>, TaskStoreError> {
        list_tasks_with_db(&self.db_path)
    }

    pub fn load_task(
        &self,
        task_id: &TaskId,
    ) -> Result<Option<(Task, Vec<TaskImage>)>, TaskStoreError> {
        load_task_with_db(&self.db_path, task_id)
    }

    pub fn create_task(
        &self,
        project_path: &Path,
        body: &str,
        image_paths: &[PathBuf],
    ) -> Result<TaskId, TaskStoreError> {
        create_task_with_db(&self.db_path, project_path, body, image_paths)
    }

    pub fn delete_task(&self, task_id: &TaskId) -> Result<bool, TaskStoreError> {
        delete_task_with_db(&self.db_path, task_id)
    }
}

pub fn resolve_tasks_db_path() -> Result<PathBuf, TaskStoreError> {
    if let Ok(value) = std::env::var("CCBOX_TASKS_DB") {
        let path = PathBuf::from(value.trim());
        return Ok(path);
    }

    let state_dir = super::resolve_ccbox_state_dir()?;
    Ok(state_dir.join("tasks.db"))
}

pub fn ensure_tasks_db_ready(db_path: &Path) -> Result<(), TaskStoreError> {
    let parent = db_path.parent().unwrap_or(db_path);
    fs::create_dir_all(parent).map_err(|error| TaskStoreError::CreateStateDir {
        path: parent.display().to_string(),
        source: error,
    })?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .map_err(|error| TaskStoreError::Migrate(error.to_string()))?;

    runtime
        .block_on(async {
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(db_path)
                .create_if_missing(true);
            let mut conn = sqlx::SqliteConnection::connect_with(&options)
                .await
                .map_err(|error| error.to_string())?;

            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(&mut conn)
                .await
                .map_err(|error| error.to_string())?;

            TASKS_MIGRATOR
                .run(&mut conn)
                .await
                .map_err(|error| error.to_string())?;

            Ok::<(), String>(())
        })
        .map_err(TaskStoreError::Migrate)?;

    Ok(())
}

fn open_tasks_connection(db_path: &Path) -> Result<Connection, TaskStoreError> {
    let conn = Connection::open(db_path).map_err(|error| TaskStoreError::OpenDb {
        path: db_path.display().to_string(),
        source: error,
    })?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    let _ = conn.busy_timeout(Duration::from_millis(250));
    Ok(conn)
}

fn list_tasks_with_db(db_path: &Path) -> Result<Vec<TaskListEntry>, TaskStoreError> {
    ensure_tasks_db_ready(db_path)?;
    let conn = open_tasks_connection(db_path)?;

    let mut stmt = conn.prepare(
        "SELECT \
            tasks.id, \
            tasks.project_path, \
            tasks.body, \
            tasks.created_at_unix_ms, \
            tasks.updated_at_unix_ms, \
            (SELECT COUNT(1) FROM task_images WHERE task_images.task_id = tasks.id) AS image_count \
         FROM tasks \
         ORDER BY tasks.updated_at_unix_ms DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let project_path: String = row.get(1)?;
        let body: String = row.get(2)?;
        let created_at_ms: i64 = row.get(3)?;
        let updated_at_ms: i64 = row.get(4)?;
        let image_count: i64 = row.get(5)?;

        Ok(TaskListEntry {
            task: Task {
                id: TaskId::new(id),
                project_path: PathBuf::from(project_path),
                body,
                created_at: unix_ms_to_system_time(created_at_ms),
                updated_at: unix_ms_to_system_time(updated_at_ms),
            },
            image_count: u32::try_from(image_count).unwrap_or(0),
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn load_task_with_db(
    db_path: &Path,
    task_id: &TaskId,
) -> Result<Option<(Task, Vec<TaskImage>)>, TaskStoreError> {
    ensure_tasks_db_ready(db_path)?;
    let conn = open_tasks_connection(db_path)?;

    let task: Option<Task> = conn
        .query_row(
            "SELECT id, project_path, body, created_at_unix_ms, updated_at_unix_ms \
             FROM tasks WHERE id = ?1",
            [task_id.to_string()],
            |row| {
                let id: String = row.get(0)?;
                let project_path: String = row.get(1)?;
                let body: String = row.get(2)?;
                let created_at_ms: i64 = row.get(3)?;
                let updated_at_ms: i64 = row.get(4)?;
                Ok(Task {
                    id: TaskId::new(id),
                    project_path: PathBuf::from(project_path),
                    body,
                    created_at: unix_ms_to_system_time(created_at_ms),
                    updated_at: unix_ms_to_system_time(updated_at_ms),
                })
            },
        )
        .optional()?;

    let Some(task) = task else {
        return Ok(None);
    };

    let mut stmt = conn.prepare(
        "SELECT task_id, ordinal, source_path, added_at_unix_ms \
         FROM task_images WHERE task_id = ?1 ORDER BY ordinal ASC",
    )?;
    let images = stmt
        .query_map([task_id.to_string()], |row| {
            let tid: String = row.get(0)?;
            let ordinal: i64 = row.get(1)?;
            let source_path: String = row.get(2)?;
            let added_at_ms: i64 = row.get(3)?;
            Ok(TaskImage {
                task_id: TaskId::new(tid),
                ordinal: u32::try_from(ordinal).unwrap_or(0),
                source_path: PathBuf::from(source_path),
                added_at: unix_ms_to_system_time(added_at_ms),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(Some((task, images)))
}

fn create_task_with_db(
    db_path: &Path,
    project_path: &Path,
    body: &str,
    image_paths: &[PathBuf],
) -> Result<TaskId, TaskStoreError> {
    ensure_tasks_db_ready(db_path)?;
    let mut conn = open_tasks_connection(db_path)?;
    let tx = conn.transaction()?;
    let now_ms = system_time_to_unix_ms(SystemTime::now());

    let id = TaskId::new(Uuid::new_v4().to_string());
    insert_task_row(&tx, &id, project_path, body, now_ms)?;
    insert_task_images(&tx, &id, image_paths, now_ms)?;
    tx.commit()?;

    Ok(id)
}

fn delete_task_with_db(db_path: &Path, task_id: &TaskId) -> Result<bool, TaskStoreError> {
    ensure_tasks_db_ready(db_path)?;
    let conn = open_tasks_connection(db_path)?;
    let affected = conn.execute("DELETE FROM tasks WHERE id = ?1", [task_id.to_string()])?;
    Ok(affected > 0)
}

fn insert_task_row(
    tx: &Transaction<'_>,
    id: &TaskId,
    project_path: &Path,
    body: &str,
    now_ms: i64,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        "INSERT INTO tasks (id, project_path, body, created_at_unix_ms, updated_at_unix_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            id.to_string(),
            project_path.display().to_string(),
            body,
            now_ms,
            now_ms
        ],
    )?;
    Ok(())
}

fn insert_task_images(
    tx: &Transaction<'_>,
    id: &TaskId,
    image_paths: &[PathBuf],
    now_ms: i64,
) -> Result<(), rusqlite::Error> {
    for (idx, path) in image_paths.iter().enumerate() {
        let ordinal = i64::try_from(idx.saturating_add(1)).unwrap_or(i64::MAX);
        tx.execute(
            "INSERT INTO task_images (task_id, ordinal, source_path, added_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4)",
            params![id.to_string(), ordinal, path.display().to_string(), now_ms],
        )?;
    }
    Ok(())
}

fn unix_ms_to_system_time(ms: i64) -> SystemTime {
    if ms <= 0 {
        return UNIX_EPOCH;
    }
    UNIX_EPOCH + Duration::from_millis(ms as u64)
}

fn system_time_to_unix_ms(time: SystemTime) -> i64 {
    let delta = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    i64::try_from(delta.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_list_load_delete_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let store = TaskStore::open(dir.path().join("tasks.db")).expect("open");

        let project_path = PathBuf::from("/tmp/project");
        let body = "Do the thing\nMore details\n";
        let images = vec![PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")];

        let task_id = store
            .create_task(&project_path, body, &images)
            .expect("create");

        let tasks = store.list_tasks().expect("list");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task.id.to_string(), task_id.to_string());
        assert_eq!(tasks[0].image_count, 2);

        let loaded = store.load_task(&task_id).expect("load").expect("present");
        assert_eq!(loaded.0.project_path, project_path);
        assert_eq!(loaded.0.body, body);
        assert_eq!(loaded.1.len(), 2);
        assert_eq!(loaded.1[0].ordinal, 1);
        assert_eq!(loaded.1[1].ordinal, 2);

        assert!(store.delete_task(&task_id).expect("delete"));
        assert!(store.list_tasks().expect("list after").is_empty());
    }
}
