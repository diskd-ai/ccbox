use std::path::Path;

#[derive(Clone, Debug)]
pub struct SpawnAgentSessionOutcome {
    pub prompt_chars: usize,
}

pub fn spawn_agent_session(
    _sessions_dir: &Path,
    _project_path: &Path,
    prompt: &str,
) -> SpawnAgentSessionOutcome {
    // TODO: Integrate with Codex to start a new agent session and persist it to the sessions dir.
    SpawnAgentSessionOutcome {
        prompt_chars: prompt.chars().count(),
    }
}
