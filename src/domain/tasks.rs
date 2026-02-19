use std::fmt;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub id: TaskId,
    pub project_path: PathBuf,
    pub body: String,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskImage {
    pub task_id: TaskId,
    pub ordinal: u32,
    pub source_path: PathBuf,
    pub added_at: SystemTime,
}

pub fn derive_task_title(body: &str) -> String {
    const MAX_CHARS: usize = 120;

    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        return trimmed.chars().take(MAX_CHARS).collect::<String>();
    }

    "(untitled)".to_string()
}

pub fn format_task_spawn_prompt(task: &Task, images: &[TaskImage]) -> String {
    let mut out = String::new();
    out.push_str(&task.body);

    if !images.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str("Attached images:\n");
        for image in images {
            out.push_str(&format!(
                "[Image {}] {}\n",
                image.ordinal,
                image.source_path.display()
            ));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_task_title_picks_first_non_empty_line() {
        let body = "\n\n  \nFirst line\nSecond line";
        assert_eq!(derive_task_title(body), "First line");
    }

    #[test]
    fn derive_task_title_falls_back_when_empty() {
        assert_eq!(derive_task_title(" \n\t\n"), "(untitled)");
    }

    #[test]
    fn format_task_spawn_prompt_appends_images_footer() {
        let task = Task {
            id: TaskId::new("t1".to_string()),
            project_path: PathBuf::from("/tmp/project"),
            body: "Hello\n".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            updated_at: SystemTime::UNIX_EPOCH,
        };
        let images = vec![
            TaskImage {
                task_id: TaskId::new("t1".to_string()),
                ordinal: 1,
                source_path: PathBuf::from("/tmp/a.png"),
                added_at: SystemTime::UNIX_EPOCH,
            },
            TaskImage {
                task_id: TaskId::new("t1".to_string()),
                ordinal: 2,
                source_path: PathBuf::from("/tmp/b.png"),
                added_at: SystemTime::UNIX_EPOCH,
            },
        ];

        let prompt = format_task_spawn_prompt(&task, &images);
        assert!(prompt.contains("Hello\n"));
        assert!(prompt.contains("Attached images:\n"));
        assert!(prompt.contains("[Image 1] /tmp/a.png\n"));
        assert!(prompt.contains("[Image 2] /tmp/b.png\n"));
    }
}
