//! Session-per-task tmux runtime.
//!
//! Each task gets its own tmux session with 4 windows:
//!   0: nvim   – editor
//!   1: lazygit – git UI
//!   2: claude  – the agent
//!   3: shell   – general purpose
//!
//! This isolates tasks from each other (separate sessions) while keeping
//! the dashboard in the "exo" session.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::primitives::{PaneId, WindowId};
use crate::runtime::{
    self, LaunchConfig, Runtime, SkillPermissions, SpawnResult, resolve_binary, tmux_cmd,
};

pub struct TmuxSessionRuntime;

impl TmuxSessionRuntime {
    fn tmux_cmd(&self, args: &[&str]) -> anyhow::Result<String> {
        tmux_cmd(args)
    }

    /// Create a dedicated tmux session for a task with 4 windows:
    ///   0: nvim   – editor
    ///   1: lazygit – git UI
    ///   2: claude  – the agent
    ///   3: shell   – general purpose
    fn launch_agent_session(
        &self,
        task_name: &str,
        task_id_short: &str,
        work_dir: &Path,
        claude_cmd: &str,
    ) -> anyhow::Result<SpawnResult> {
        let work_dir_str = work_dir.display().to_string();
        let session_name = sanitize_tmux_session_name(task_name, task_id_short);

        // Window 0: nvim (created with the session)
        self.tmux_cmd(&[
            "new-session",
            "-d",
            "-s",
            &session_name,
            "-n",
            "nvim",
            "-c",
            &work_dir_str,
        ])?;
        self.tmux_cmd(&[
            "send-keys",
            "-t",
            &format!("{session_name}:nvim"),
            "nvim .",
            "Enter",
        ])?;

        // Window 1: lazygit
        self.tmux_cmd(&[
            "new-window",
            "-t",
            &session_name,
            "-n",
            "lazygit",
            "-c",
            &work_dir_str,
        ])?;
        self.tmux_cmd(&[
            "send-keys",
            "-t",
            &format!("{session_name}:lazygit"),
            "lazygit",
            "Enter",
        ])?;

        // Window 2: claude (the agent)
        self.tmux_cmd(&[
            "new-window",
            "-t",
            &session_name,
            "-n",
            "claude",
            "-c",
            &work_dir_str,
        ])?;
        let claude_pane = self.tmux_cmd(&[
            "list-panes",
            "-t",
            &format!("{session_name}:claude"),
            "-F",
            "#{pane_id}",
        ])?;
        self.tmux_cmd(&[
            "send-keys",
            "-t",
            &format!("{session_name}:claude"),
            "-l",
            claude_cmd,
        ])?;
        self.tmux_cmd(&[
            "send-keys",
            "-t",
            &format!("{session_name}:claude"),
            "Enter",
        ])?;

        // Window 3: shell
        self.tmux_cmd(&[
            "new-window",
            "-t",
            &session_name,
            "-n",
            "shell",
            "-c",
            &work_dir_str,
        ])?;

        // Focus the claude window by default
        self.tmux_cmd(&["select-window", "-t", &format!("{session_name}:claude")])?;

        Ok(SpawnResult {
            window_id: WindowId::from(session_name),
            pane_id: PaneId::from(claude_pane),
        })
    }
}

impl Runtime for TmuxSessionRuntime {
    // ── Delegate git/worktree methods to TmuxRuntime (identical logic) ──

    fn create_worktree(
        &self,
        repo_root: &Path,
        name: &str,
        perms: &SkillPermissions,
        branch: Option<&str>,
        hooks_source: &Path,
        jwt_token: &str,
    ) -> anyhow::Result<PathBuf> {
        runtime::TmuxRuntime.create_worktree(
            repo_root,
            name,
            perms,
            branch,
            hooks_source,
            jwt_token,
        )
    }

    fn recreate_worktree(
        &self,
        repo_root: &Path,
        work_dir: &Path,
        jwt_token: &str,
    ) -> anyhow::Result<()> {
        runtime::TmuxRuntime.recreate_worktree(repo_root, work_dir, jwt_token)
    }

    fn setup_dir_config(
        &self,
        hooks_source: &Path,
        work_dir: &Path,
        perms: &SkillPermissions,
        jwt_token: &str,
    ) -> anyhow::Result<()> {
        runtime::TmuxRuntime.setup_dir_config(hooks_source, work_dir, perms, jwt_token)
    }

    fn init_scratch_dir(&self, scratch_dir: &Path) -> anyhow::Result<()> {
        runtime::TmuxRuntime.init_scratch_dir(scratch_dir)
    }

    fn send_keys_to_pane(&self, pane_id: &str, message: &str) -> anyhow::Result<()> {
        runtime::TmuxRuntime.send_keys_to_pane(pane_id, message)
    }

    fn capture_pane_output(&self, pane_id: &str) -> anyhow::Result<String> {
        runtime::TmuxRuntime.capture_pane_output(pane_id)
    }

    fn remove_worktree(&self, path: &Path) -> anyhow::Result<()> {
        runtime::TmuxRuntime.remove_worktree(path)
    }

    // ── Session-specific implementations ────────────────────────────────

    fn launch_agent(&self, config: LaunchConfig) -> anyhow::Result<SpawnResult> {
        let claude_bin = resolve_binary("claude")?;

        let claude_dir = config.work_dir.join(".claude");
        std::fs::create_dir_all(&claude_dir)?;

        let mut script = format!("#!/bin/sh\nunset CLAUDECODE\nexec {claude_bin}");
        if config.skip_permissions {
            script.push_str(" --dangerously-skip-permissions");
        }
        script.push_str(&format!(" --session-id {}", config.session_id));

        if let Some(user_prompt) = config.user_prompt {
            // Full mode: write user prompt to file, use --system-prompt
            std::fs::write(claude_dir.join("prompt.txt"), user_prompt)?;
            script.push_str(" \"$(cat .claude/prompt.txt)\"");
            if let Some(sys) = config.system_prompt {
                std::fs::write(claude_dir.join("system-prompt.txt"), sys)?;
                script.push_str(" --system-prompt \"$(cat .claude/system-prompt.txt)\"");
            }
        } else {
            // Interactive mode: idle prompt, use --append-system-prompt
            std::fs::write(
                claude_dir.join("idle-prompt.txt"),
                "Await further instructions.",
            )?;
            script.push_str(" \"$(cat .claude/idle-prompt.txt)\"");
            if let Some(sys) = config.system_prompt {
                std::fs::write(claude_dir.join("system-prompt.txt"), sys)?;
                script.push_str(" --append-system-prompt \"$(cat .claude/system-prompt.txt)\"");
            }
        }

        let script_path = claude_dir.join("launch.sh");
        std::fs::write(&script_path, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))?;
        }

        self.launch_agent_session(
            config.task_name,
            config.task_id_short,
            config.work_dir,
            "sh .claude/launch.sh",
        )
    }

    fn resume_agent(
        &self,
        task_name: &str,
        task_id_short: &str,
        session_id: &str,
        work_dir: &Path,
        skip_permissions: bool,
    ) -> anyhow::Result<SpawnResult> {
        let claude_bin = resolve_binary("claude")?;
        let skip_flag = if skip_permissions {
            " --dangerously-skip-permissions"
        } else {
            ""
        };
        let claude_cmd = format!("env -u CLAUDECODE {claude_bin}{skip_flag} --resume {session_id}");

        self.launch_agent_session(task_name, task_id_short, work_dir, &claude_cmd)
    }

    fn relaunch_agent(
        &self,
        task_name: &str,
        task_id_short: &str,
        work_dir: &Path,
    ) -> anyhow::Result<SpawnResult> {
        self.launch_agent_session(task_name, task_id_short, work_dir, "sh .claude/launch.sh")
    }

    fn kill_task_env(&self, session_name: &str) -> anyhow::Result<()> {
        self.tmux_cmd(&["kill-session", "-t", session_name])?;
        Ok(())
    }

    fn select_task_env(&self, session_name: &str) -> anyhow::Result<()> {
        self.tmux_cmd(&["switch-client", "-t", session_name])?;
        Ok(())
    }
}

/// Sanitize a task name into a valid, unique tmux session name.
/// Tmux session names cannot contain dots or colons. We include the task ID
/// short hash to guarantee uniqueness even when multiple tasks share a name.
fn sanitize_tmux_session_name(task_name: &str, task_id_short: &str) -> String {
    let clean: String = task_name
        .chars()
        .map(|c| if c == '.' || c == ':' { '-' } else { c })
        .collect();
    format!("cc-{clean}-{task_id_short}")
}

/// Returns the set of active tmux session names.
/// Each task gets its own session, so we check which sessions exist.
pub fn active_task_envs() -> HashSet<WindowId> {
    let mut set = HashSet::new();
    if let Ok(output) = tmux_cmd(&["list-sessions", "-F", "#{session_name}"]) {
        for line in output.lines() {
            let name = line.trim();
            if !name.is_empty() {
                set.insert(WindowId::from(name.to_string()));
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_dots_and_colons() {
        assert_eq!(
            sanitize_tmux_session_name("my.task:v2", "abc123"),
            "cc-my-task-v2-abc123"
        );
    }

    #[test]
    fn sanitize_preserves_clean_names() {
        assert_eq!(
            sanitize_tmux_session_name("deploy", "def456"),
            "cc-deploy-def456"
        );
    }
}
