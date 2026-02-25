-- Tasks persistence (v1)
--
-- Notes:
-- - `created_at_unix_ms` / `updated_at_unix_ms` use milliseconds since UNIX epoch.
-- - Image ordinals are stable and never renumbered.

CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY NOT NULL,
  project_path TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at_unix_ms INTEGER NOT NULL,
  updated_at_unix_ms INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tasks_updated_at ON tasks(updated_at_unix_ms);
CREATE INDEX IF NOT EXISTS idx_tasks_project_path ON tasks(project_path);

CREATE TABLE IF NOT EXISTS task_images (
  task_id TEXT NOT NULL,
  ordinal INTEGER NOT NULL,
  source_path TEXT NOT NULL,
  added_at_unix_ms INTEGER NOT NULL,
  PRIMARY KEY (task_id, ordinal),
  FOREIGN KEY(task_id) REFERENCES tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_task_images_task_id ON task_images(task_id);

