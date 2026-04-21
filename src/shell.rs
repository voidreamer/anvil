//! Shell spawning and detection

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result};

/// Prefix used for every anvil-managed shim tempdir so the sweeper can find
/// orphans reliably.
pub const SHIM_DIR_PREFIX: &str = "anvil-shell-";

/// Detect the user's preferred shell
pub fn detect_shell() -> String {
    // Check SHELL environment variable
    if let Ok(shell) = std::env::var("SHELL") {
        return shell;
    }
    
    // Platform defaults
    if cfg!(target_os = "windows") {
        // Check for PowerShell first
        if which::which("pwsh").is_ok() {
            return "pwsh".to_string();
        }
        return "cmd".to_string();
    }
    
    // Unix default
    "bash".to_string()
}

/// Spawn an interactive shell with the given environment
pub fn spawn_shell(shell: &str, env: &HashMap<String, String>) -> Result<()> {
    let shell_name = std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(shell);
    
    println!("Starting {} shell with resolved environment...", shell_name);
    println!("Type 'exit' to return to your original shell.\n");
    
    let mut cmd = Command::new(shell);
    
    // Set up environment
    cmd.env_clear();
    cmd.envs(env);
    
    // Add anvil indicator to prompt
    if let Some(prompt) = env.get("PS1") {
        let new_prompt = format!("[anvil] {}", prompt);
        cmd.env("PS1", new_prompt);
    } else {
        // Set a simple prompt for bash
        cmd.env("PS1", "[anvil] \\u@\\h:\\w\\$ ");
    }
    
    // Platform-specific setup
    cfg_if::cfg_if! {
        if #[cfg(unix)] {
            use std::os::unix::process::CommandExt;
            // Replace current process with shell
            let err = cmd.exec();
            Err(err.into())
        } else {
            // Windows: spawn and wait
            let status = cmd.status()?;
            if !status.success() {
                anyhow::bail!("Shell exited with status: {:?}", status.code());
            }
            Ok(())
        }
    }
}

/// Generate shell-specific environment setup script
pub fn generate_env_script(shell: &str, env: &HashMap<String, String>) -> String {
    let shell_name = std::path::Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(shell);
    
    match shell_name {
        "bash" | "sh" | "zsh" => {
            let mut script = String::new();
            for (key, value) in env {
                // Escape special characters
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                script.push_str(&format!("export {}=\"{}\"\n", key, escaped));
            }
            script
        }
        "fish" => {
            let mut script = String::new();
            for (key, value) in env {
                let escaped = value.replace('\\', "\\\\").replace('\'', "\\'");
                script.push_str(&format!("set -gx {} '{}'\n", key, escaped));
            }
            script
        }
        "pwsh" | "powershell" => {
            let mut script = String::new();
            for (key, value) in env {
                let escaped = value.replace('\'', "''");
                script.push_str(&format!("$env:{} = '{}'\n", key, escaped));
            }
            script
        }
        "cmd" => {
            let mut script = String::new();
            for (key, value) in env {
                script.push_str(&format!("set {}={}\n", key, value));
            }
            script
        }
        _ => {
            // Default to bash-style
            let mut script = String::new();
            for (key, value) in env {
                let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
                script.push_str(&format!("export {}=\"{}\"\n", key, escaped));
            }
            script
        }
    }
}

/// Write a PATH shim for each `(alias, command)` pair into a fresh tempdir
/// and return the path.  The tempdir is *leaked* on purpose — its lifetime
/// is the interactive subshell the caller is about to spawn, and the sweeper
/// reclaims it on the next `anvil shell` invocation.
///
/// On POSIX the shim is a `chmod 755` shebang script; on Windows it's a
/// `.cmd` wrapper resolvable through PATHEXT by cmd.exe, PowerShell, and
/// pwsh alike.
pub fn materialize_commands(commands: &HashMap<String, String>) -> Result<PathBuf> {
    let dir = tempfile::Builder::new()
        .prefix(SHIM_DIR_PREFIX)
        .tempdir()
        .context("Failed to create shim tempdir")?
        // Disown so it survives the `exec` into the user's shell.  The
        // sweeper below reclaims it on the next `anvil shell` invocation.
        .keep();

    for (alias, target) in commands {
        write_shim(&dir, alias, target)
            .with_context(|| format!("Failed to write shim for {:?}", alias))?;
    }

    Ok(dir)
}

#[cfg(unix)]
fn write_shim(dir: &Path, alias: &str, target: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // `target` is already the expanded command string — e.g. a bare path or
    // `"/Applications/... Painter" --flag`.  `exec "$@"` with the command
    // embedded raw preserves any baked-in arguments and lets the user append
    // their own.
    let script = format!("#!/usr/bin/env bash\nexec {} \"$@\"\n", target);
    let path = dir.join(alias);
    std::fs::write(&path, script)?;
    let mut perms = std::fs::metadata(&path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms)?;
    Ok(())
}

#[cfg(windows)]
fn write_shim(dir: &Path, alias: &str, target: &str) -> Result<()> {
    // `.cmd` is resolvable through PATHEXT by cmd.exe, PowerShell, and pwsh.
    // `%*` forwards every argument the user typed.
    let script = format!("@echo off\r\n{} %*\r\n", target);
    let path = dir.join(format!("{}.cmd", alias));
    std::fs::write(&path, script)?;
    Ok(())
}

/// Delete `anvil-shell-*` tempdirs inside `root` whose mtime is older than
/// `ttl`.  Runs before materialising the current session's shims so a
/// SIGKILL'd shell never leaks more than one orphan.
///
/// Failures are best-effort: a file we can't stat or unlink is skipped
/// rather than aborting the shell entry.
pub fn sweep_stale_shims_in(root: &Path, ttl: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = SystemTime::now();

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else { continue };
        if !name_str.starts_with(SHIM_DIR_PREFIX) {
            continue;
        }

        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() {
            continue;
        }
        let Ok(mtime) = meta.modified() else { continue };
        let Ok(age) = now.duration_since(mtime) else { continue };
        if age < ttl {
            continue;
        }

        let _ = std::fs::remove_dir_all(entry.path());
    }
}

/// Default sweep: walks the system temp dir.
pub fn sweep_stale_shims(ttl: std::time::Duration) {
    sweep_stale_shims_in(&std::env::temp_dir(), ttl);
}

/// Prepend `shim_dir` to the `PATH` entry in `env`, creating the entry if
/// absent.  Separator is platform-native (`:` on Unix, `;` on Windows).
pub fn prepend_path(env: &mut HashMap<String, String>, shim_dir: &Path) {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let shim_str = shim_dir.to_string_lossy().into_owned();
    let new_path = match env.get("PATH") {
        Some(existing) if !existing.is_empty() => format!("{}{}{}", shim_str, sep, existing),
        _ => shim_str,
    };
    env.insert("PATH".to_string(), new_path);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn materialize_writes_shims() {
        let mut cmds = HashMap::new();
        cmds.insert("hello".to_string(), "/bin/echo hi".to_string());
        let dir = materialize_commands(&cmds).unwrap();

        #[cfg(unix)]
        let shim = dir.join("hello");
        #[cfg(windows)]
        let shim = dir.join("hello.cmd");

        assert!(shim.exists(), "shim file should exist at {:?}", shim);
        let content = std::fs::read_to_string(&shim).unwrap();
        assert!(content.contains("/bin/echo hi"), "content was: {}", content);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&shim).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn shim_roundtrip_executes_on_unix() {
        // This test exercises the actual shim as an executable; Windows
        // .cmd invocation is harder to drive from Rust tests, so restrict
        // to Unix where `/bin/echo` is reliably present.
        #[cfg(unix)]
        {
            let mut cmds = HashMap::new();
            cmds.insert("anvil_test_echo".to_string(), "/bin/echo ok".to_string());
            let dir = materialize_commands(&cmds).unwrap();
            let out = std::process::Command::new(dir.join("anvil_test_echo"))
                .arg("extra")
                .output()
                .expect("shim should be executable");
            assert!(out.status.success());
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(stdout.trim() == "ok extra", "got: {:?}", stdout);
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn sweep_removes_stale_dirs() {
        // Isolate to a private root so a parallel test doesn't lose its dir.
        let root = tempfile::tempdir().unwrap();
        let stale = tempfile::Builder::new()
            .prefix(SHIM_DIR_PREFIX)
            .tempdir_in(root.path())
            .unwrap()
            .keep();
        assert!(stale.exists());
        sweep_stale_shims_in(root.path(), Duration::from_secs(0));
        assert!(!stale.exists(), "sweep should have deleted {:?}", stale);
    }

    #[test]
    fn sweep_keeps_fresh_dirs() {
        let root = tempfile::tempdir().unwrap();
        let fresh = tempfile::Builder::new()
            .prefix(SHIM_DIR_PREFIX)
            .tempdir_in(root.path())
            .unwrap()
            .keep();
        sweep_stale_shims_in(root.path(), Duration::from_secs(3600));
        assert!(fresh.exists());
    }

    #[test]
    fn sweep_ignores_non_anvil_dirs() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::Builder::new()
            .prefix("unrelated-")
            .tempdir_in(root.path())
            .unwrap()
            .keep();
        sweep_stale_shims_in(root.path(), Duration::from_secs(0));
        assert!(other.exists(), "sweep should ignore non-anvil dirs");
    }

    #[test]
    fn prepend_path_inserts_separator() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        prepend_path(&mut env, Path::new("/tmp/shim"));
        let sep = if cfg!(windows) { ';' } else { ':' };
        assert_eq!(env["PATH"], format!("/tmp/shim{}/usr/bin", sep));
    }

    #[test]
    fn prepend_path_handles_missing_path() {
        let mut env = HashMap::new();
        prepend_path(&mut env, Path::new("/tmp/shim"));
        assert_eq!(env["PATH"], "/tmp/shim");
    }
}
