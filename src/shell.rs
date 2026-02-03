//! Shell spawning and detection

use std::collections::HashMap;
use std::process::Command;

use anyhow::Result;

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
