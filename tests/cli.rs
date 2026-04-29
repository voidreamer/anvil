use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

/// Helper: create a temp directory with packages and an anvil config pointing
/// to it.  Returns (TempDir, config_path).
fn setup_env() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    // python-3.11
    let python_dir = pkg_dir.join("python/3.11");
    fs::create_dir_all(&python_dir).unwrap();
    fs::write(
        python_dir.join("package.yaml"),
        r#"
name: python
version: "3.11"
description: Python 3.11
environment:
  PYTHON_VERSION: "3.11"
  PATH: ${PACKAGE_ROOT}/bin:${PATH}
commands:
  python: ${PACKAGE_ROOT}/bin/python3.11
"#,
    )
    .unwrap();

    // maya-2024 (flat file)
    fs::write(
        pkg_dir.join("maya-2024.yaml"),
        r#"
name: maya
version: "2024"
description: Autodesk Maya 2024
requires:
  - python-3.11
environment:
  MAYA_VERSION: "2024"
  MAYA_LOCATION: /usr/autodesk/maya2024
  PATH: ${MAYA_LOCATION}/bin:${PATH}
commands:
  maya: ${MAYA_LOCATION}/bin/maya
"#,
    )
    .unwrap();

    // studio-blender-tools-1.0.0 (hyphenated name)
    let sbt_dir = pkg_dir.join("studio-blender-tools/1.0.0");
    fs::create_dir_all(&sbt_dir).unwrap();
    fs::write(
        sbt_dir.join("package.yaml"),
        r#"
name: studio-blender-tools
version: "1.0.0"
description: Studio Blender addons
environment:
  STUDIO_TOOLS: enabled
"#,
    )
    .unwrap();

    // config
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!(
            "package_paths:\n  - {}\naliases:\n  maya-full: [maya-2024]\n",
            pkg_dir.display()
        ),
    )
    .unwrap();

    let config_str = config_path.to_string_lossy().to_string();
    (dir, config_str)
}

fn anvil(config: &str) -> Command {
    let mut cmd = Command::cargo_bin("anvil").unwrap();
    cmd.env("ANVIL_CONFIG", config);
    cmd.env("RUST_LOG", "anvil=error"); // suppress info logs in tests
    cmd
}

// ---- anvil list ----

#[test]
fn list_all_packages() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya"))
        .stdout(predicate::str::contains("python"))
        .stdout(predicate::str::contains("studio-blender-tools"));
}

#[test]
fn list_versions() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["list", "maya"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2024"));
}

// ---- anvil info ----

#[test]
fn info_simple() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["info", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Name: maya"))
        .stdout(predicate::str::contains("Version: 2024"))
        .stdout(predicate::str::contains("python-3.11"));
}

#[test]
fn info_hyphenated_name() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["info", "studio-blender-tools-1.0.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Name: studio-blender-tools"));
}

// ---- anvil env ----

#[test]
fn env_key_value() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["env", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MAYA_VERSION=2024"))
        .stdout(predicate::str::contains("PYTHON_VERSION=3.11"));
}

#[test]
fn env_json() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["env", "maya-2024", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"MAYA_VERSION\""))
        .stdout(predicate::str::contains("\"2024\""));
}

#[test]
fn env_export() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["env", "maya-2024", "--export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("export MAYA_VERSION=\"2024\""));
}

// ---- anvil validate ----

#[test]
fn validate_all() {
    // Test fixtures use fictional command target paths
    // (e.g. /usr/autodesk/maya2024/bin/maya), so validate reports
    // command warnings but still succeeds.  Dependencies do resolve.
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["validate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("All dependencies resolve").or(predicate::str::contains("All packages valid!")));
}

#[test]
fn validate_single() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["validate", "maya-2024"])
        .assert()
        .success();
}

// ---- alias resolution ----

#[test]
fn alias_resolves() {
    let (_dir, cfg) = setup_env();
    // maya-full alias should resolve maya-2024 (which pulls in python-3.11)
    anvil(&cfg)
        .args(["env", "maya-full"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MAYA_VERSION=2024"))
        .stdout(predicate::str::contains("PYTHON_VERSION=3.11"));
}

// ---- error cases ----

#[test]
fn unknown_package_fails() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["env", "nonexistent-1.0"])
        .assert()
        .failure();
}

#[test]
fn run_without_command_fails() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["run", "maya-2024", "--"])
        .assert()
        .failure();
}

#[test]
fn run_multi_token_command_alias() {
    // A command alias whose value has whitespace (program + baked-in
    // args) must be split so the user's extra args land after the
    // baked-in ones. Regression for issue where the whole string was
    // passed to Command::new() as a single filename.
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("greeter-1.0.yaml"),
        "name: greeter\nversion: \"1.0\"\ncommands:\n  greet: /bin/echo hello from\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    anvil(&config_path.to_string_lossy())
        .args(["run", "greeter-1.0", "--", "greet", "world"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello from world"));
}

#[test]
fn run_tilde_expands_in_each_token() {
    // ~/ should expand in every token, not just when it's the leading
    // character of the whole resolved value. Regression for aliases like:
    //     usdview: python3.14 ~/USD/bin/usdview
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    // Make a real target file under $HOME so ~/ expansion is observable.
    // We write to $TMPDIR-style path the test controls by overriding HOME.
    let fake_home = dir.path().join("home");
    fs::create_dir_all(fake_home.join("bin")).unwrap();
    let script = fake_home.join("bin/ping.sh");
    fs::write(&script, "#!/bin/bash\necho PONG\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }

    fs::write(
        pkg_dir.join("pingpkg-1.0.yaml"),
        "name: pingpkg\nversion: \"1.0\"\ncommands:\n  ping: /bin/bash ~/bin/ping.sh\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    anvil(&config_path.to_string_lossy())
        .env("HOME", fake_home.to_str().unwrap())
        .args(["run", "pingpkg-1.0", "--", "ping"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PONG"));
}

#[test]
fn run_quoted_tokens_in_alias() {
    // Quoted substrings in an alias must stay as a single argv element.
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("qtest-1.0.yaml"),
        "name: qtest\nversion: \"1.0\"\ncommands:\n  say: /bin/echo \"hello world\"\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    anvil(&config_path.to_string_lossy())
        .args(["run", "qtest-1.0", "--", "say"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello world"));
}

// ---- flat file + nested coexistence ----

#[test]
fn flat_and_nested_coexist() {
    let (_dir, cfg) = setup_env();
    // maya is flat, python is nested — both should appear
    anvil(&cfg)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya"))
        .stdout(predicate::str::contains("python"));
}

// ---- per-project config ----

#[test]
fn project_config_loaded() {
    let dir = TempDir::new().unwrap();
    let project_dir = dir.path().join("myproject");
    fs::create_dir_all(&project_dir).unwrap();

    // Project-local packages
    let pkg_dir = project_dir.join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("localtools-1.0.yaml"),
        "name: localtools\nversion: \"1.0\"\nenvironment:\n  LOCAL: yes\n",
    )
    .unwrap();

    // Project .anvil.yaml
    fs::write(
        project_dir.join(".anvil.yaml"),
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    // Empty global config (no packages)
    let global_cfg = dir.path().join("global.yaml");
    fs::write(&global_cfg, "package_paths: []\n").unwrap();

    Command::cargo_bin("anvil")
        .unwrap()
        .env("ANVIL_CONFIG", global_cfg.to_str().unwrap())
        .env("RUST_LOG", "anvil=error")
        .current_dir(&project_dir)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("localtools"));
}

// ---- anvil lock ----

#[test]
fn lock_creates_lockfile() {
    let (dir, cfg) = setup_env();
    let lock_path = dir.path().join("anvil.lock");

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Locked"))
        .stdout(predicate::str::contains("maya"));

    assert!(lock_path.exists());
    let content = fs::read_to_string(&lock_path).unwrap();
    assert!(content.contains("maya"));
    assert!(content.contains("python"));
}

#[test]
fn lock_pins_versions() {
    let (dir, cfg) = setup_env();

    // Lock maya-2024
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    // Subsequent resolve should use the lockfile
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["env", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MAYA_VERSION=2024"));
}

// ---- anvil context ----

#[test]
fn context_save_and_show() {
    let (dir, cfg) = setup_env();
    let ctx_path = dir.path().join("test.ctx.json");

    // Save
    anvil(&cfg)
        .args(["context", "save", "maya-2024", "-o", ctx_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("Saved context"));

    assert!(ctx_path.exists());

    // Show
    anvil(&cfg)
        .args(["context", "show", ctx_path.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya-2024"))
        .stdout(predicate::str::contains("python-3.11"));
}

#[test]
fn context_show_json() {
    let (dir, cfg) = setup_env();
    let ctx_path = dir.path().join("test.ctx.json");

    anvil(&cfg)
        .args(["context", "save", "maya-2024", "-o", ctx_path.to_str().unwrap()])
        .assert()
        .success();

    anvil(&cfg)
        .args(["context", "show", ctx_path.to_str().unwrap(), "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"MAYA_VERSION\""));
}

#[test]
fn context_show_export() {
    let (dir, cfg) = setup_env();
    let ctx_path = dir.path().join("test.ctx.json");

    anvil(&cfg)
        .args(["context", "save", "maya-2024", "-o", ctx_path.to_str().unwrap()])
        .assert()
        .success();

    anvil(&cfg)
        .args(["context", "show", ctx_path.to_str().unwrap(), "--export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("export MAYA_VERSION=\"2024\""));
}

// ---- anvil init ----

#[test]
fn init_nested() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .current_dir(dir.path())
        .args(["init", "my-tool", "--version", "2.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created my-tool/2.0/package.yaml"));

    let pkg = dir.path().join("my-tool/2.0/package.yaml");
    assert!(pkg.exists());
    let content = fs::read_to_string(pkg).unwrap();
    assert!(content.contains("name: my-tool"));
    assert!(content.contains("version: \"2.0\""));
}

#[test]
fn init_flat() {
    let dir = TempDir::new().unwrap();

    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .current_dir(dir.path())
        .args(["init", "quick-fix", "--flat"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Created quick-fix-1.0.0.yaml"));

    let pkg = dir.path().join("quick-fix-1.0.0.yaml");
    assert!(pkg.exists());
    let content = fs::read_to_string(pkg).unwrap();
    assert!(content.contains("name: quick-fix"));
}

#[test]
fn init_refuses_overwrite() {
    let dir = TempDir::new().unwrap();

    // Create first
    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .current_dir(dir.path())
        .args(["init", "my-tool"])
        .assert()
        .success();

    // Second attempt should fail
    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .current_dir(dir.path())
        .args(["init", "my-tool"])
        .assert()
        .failure();
}

// ---- alias with spaces in path ----

#[test]
fn run_whole_path_with_spaces() {
    // When the alias value is a bare path to an existing file containing
    // spaces, it should be executed directly without being split on
    // whitespace. Regression for `/Applications/Houdini 20/bin/hython`.
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    // Create a real target file whose path contains a space.
    let spaced_dir = dir.path().join("With Space");
    fs::create_dir_all(&spaced_dir).unwrap();
    let script = spaced_dir.join("say.sh");
    fs::write(&script, "#!/bin/bash\necho whole-path-ok\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&script).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script, perms).unwrap();
    }

    fs::write(
        pkg_dir.join("spacey-1.0.yaml"),
        format!(
            "name: spacey\nversion: \"1.0\"\ncommands:\n  say: {}\n",
            script.display()
        ),
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    anvil(&config_path.to_string_lossy())
        .args(["run", "spacey-1.0", "--", "say"])
        .assert()
        .success()
        .stdout(predicate::str::contains("whole-path-ok"));
}

// ---- validate detects missing command targets ----

fn setup_broken_cmd_pkg() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("broken-1.0.yaml"),
        "name: broken\nversion: \"1.0\"\ncommands:\n  ghost: /does/not/exist/xyz\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    let cfg = config_path.to_string_lossy().to_string();
    (dir, cfg)
}

#[test]
fn validate_warns_missing_command_target() {
    let (_dir, cfg) = setup_broken_cmd_pkg();
    // Default: warnings reported but validation still succeeds.
    anvil(&cfg)
        .args(["validate", "broken-1.0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ghost"))
        .stdout(predicate::str::contains("file does not exist"));
}

#[test]
fn validate_strict_fails_on_missing_command_target() {
    let (_dir, cfg) = setup_broken_cmd_pkg();
    anvil(&cfg)
        .args(["validate", "broken-1.0", "--strict"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("ghost"));
}

// ---- --refresh flag ----

#[test]
fn refresh_flag_runs() {
    // Smoke-test: --refresh works as a global flag and produces normal output.
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["--refresh", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya"));
}

// ---- anvil completions ----

#[test]
fn completions_bash() {
    Command::cargo_bin("anvil")
        .unwrap()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_anvil"));
}

#[test]
fn completions_zsh() {
    Command::cargo_bin("anvil")
        .unwrap()
        .args(["completions", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#compdef anvil"));
}

// ---- package filters ----

#[test]
fn filter_include() {
    let (dir, _) = setup_env();
    let cfg_path = dir.path().join("filtered.yaml");
    let pkg_dir = dir.path().join("packages");

    fs::write(
        &cfg_path,
        format!(
            "package_paths:\n  - {}\nfilters:\n  include:\n    - \"maya*\"\n",
            pkg_dir.display()
        ),
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya"))
        .stdout(predicate::str::contains("python").not());
}

#[test]
fn filter_exclude() {
    let (dir, _) = setup_env();
    let cfg_path = dir.path().join("filtered.yaml");
    let pkg_dir = dir.path().join("packages");

    fs::write(
        &cfg_path,
        format!(
            "package_paths:\n  - {}\nfilters:\n  exclude:\n    - \"studio-*\"\n",
            pkg_dir.display()
        ),
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya"))
        .stdout(predicate::str::contains("studio-blender-tools").not());
}

// ---- hooks ----

#[test]
fn hooks_run_in_order() {
    let (dir, _) = setup_env();
    let cfg_path = dir.path().join("hooks.yaml");
    let pkg_dir = dir.path().join("packages");

    fs::write(
        &cfg_path,
        format!(
            r#"package_paths:
  - {}
hooks:
  pre_run:
    - echo PRE_RUN
"#,
            pkg_dir.display()
        ),
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["run", "python-3.11", "--", "echo", "COMMAND"])
        .assert()
        .success()
        .stdout(predicate::str::contains("PRE_RUN"))
        .stdout(predicate::str::contains("COMMAND"));
}

#[test]
fn hook_failure_aborts() {
    let (dir, _) = setup_env();
    let cfg_path = dir.path().join("hooks.yaml");
    let pkg_dir = dir.path().join("packages");

    fs::write(
        &cfg_path,
        format!(
            r#"package_paths:
  - {}
hooks:
  pre_run:
    - exit 1
"#,
            pkg_dir.display()
        ),
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["run", "python-3.11", "--", "echo", "should-not-run"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("should-not-run").not());
}

// ---- anvil wrap ----

#[test]
fn wrap_creates_scripts() {
    let (dir, cfg) = setup_env();
    let wrap_dir = dir.path().join("wrappers");

    anvil(&cfg)
        .args(["wrap", "maya-2024", "--dir", wrap_dir.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("wrapper"))
        .stdout(predicate::str::contains("maya"));

    // maya command should exist as a wrapper
    let maya_wrapper = wrap_dir.join("maya");
    assert!(maya_wrapper.exists());
    let content = fs::read_to_string(maya_wrapper).unwrap();
    assert!(content.contains("anvil run"));
    assert!(content.contains("maya"));
}

#[test]
fn wrap_includes_dependency_commands() {
    let (dir, cfg) = setup_env();
    let wrap_dir = dir.path().join("wrappers");

    // maya depends on python, so python commands should be in the wrappers too
    anvil(&cfg)
        .args(["wrap", "maya-2024", "--dir", wrap_dir.to_str().unwrap()])
        .assert()
        .success();

    assert!(wrap_dir.join("maya").exists());
    assert!(wrap_dir.join("python").exists());
}

// ---- anvil publish ----

#[test]
fn publish_nested() {
    let (dir, _) = setup_env();
    let target = dir.path().join("published");
    fs::create_dir_all(&target).unwrap();

    let src = dir.path().join("packages/python/3.11");

    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .args([
            "publish",
            target.to_str().unwrap(),
            "--path",
            src.to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Published python-3.11"));

    assert!(target.join("python/3.11/package.yaml").exists());
}

#[test]
fn publish_flat() {
    let (dir, _) = setup_env();
    let target = dir.path().join("published");
    fs::create_dir_all(&target).unwrap();

    let src = dir.path().join("packages/maya-2024.yaml");

    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .args([
            "publish",
            target.to_str().unwrap(),
            "--path",
            src.to_str().unwrap(),
            "--flat",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Published maya-2024"));

    assert!(target.join("maya-2024.yaml").exists());
}

#[test]
fn publish_refuses_overwrite() {
    let (dir, _) = setup_env();
    let target = dir.path().join("published");
    fs::create_dir_all(&target).unwrap();

    let src = dir.path().join("packages/python/3.11");

    // First publish
    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .args(["publish", target.to_str().unwrap(), "--path", src.to_str().unwrap()])
        .assert()
        .success();

    // Second should fail
    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .args(["publish", target.to_str().unwrap(), "--path", src.to_str().unwrap()])
        .assert()
        .failure();
}

// ---- first-run hints / init --config ----

#[test]
fn list_emits_hint_when_no_packages_found() {
    // Config exists but points at an empty directory: list should emit a
    // hint to stderr instead of staying silent.
    let dir = TempDir::new().unwrap();
    let empty_pkg_dir = dir.path().join("empty-packages");
    fs::create_dir_all(&empty_pkg_dir).unwrap();
    let cfg_path = dir.path().join("config.yaml");
    fs::write(
        &cfg_path,
        format!("package_paths:\n  - {}\n", empty_pkg_dir.display()),
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("No packages found"));
}

#[test]
fn list_hint_when_package_paths_missing() {
    // Config has package_paths entries but none exist on disk.
    let dir = TempDir::new().unwrap();
    let cfg_path = dir.path().join("config.yaml");
    fs::write(
        &cfg_path,
        "package_paths:\n  - /nonexistent/anvil-test/aaa\n  - /nonexistent/anvil-test/bbb\n",
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("None of the configured package_paths exist"));
}

#[test]
fn init_config_scaffolds_global_yaml() {
    // anvil init --config writes ~/.anvil.yaml when none exists.
    // We override HOME so the test never touches the real one.
    let dir = TempDir::new().unwrap();
    let fake_home = dir.path().join("home");
    fs::create_dir_all(&fake_home).unwrap();

    Command::cargo_bin("anvil")
        .unwrap()
        .env("HOME", &fake_home)
        .env("RUST_LOG", "anvil=error")
        .env_remove("ANVIL_CONFIG")
        .args(["init", "--config"])
        .assert()
        .success()
        .stdout(predicate::str::contains(".anvil.yaml"));

    let cfg = fake_home.join(".anvil.yaml");
    assert!(cfg.exists());
    let content = fs::read_to_string(&cfg).unwrap();
    assert!(content.contains("package_paths:"));
    assert!(content.contains("~/packages"));
}

#[test]
fn init_config_refuses_overwrite() {
    let dir = TempDir::new().unwrap();
    let fake_home = dir.path().join("home");
    fs::create_dir_all(&fake_home).unwrap();
    fs::write(fake_home.join(".anvil.yaml"), "package_paths: []\n").unwrap();

    Command::cargo_bin("anvil")
        .unwrap()
        .env("HOME", &fake_home)
        .env("RUST_LOG", "anvil=error")
        .env_remove("ANVIL_CONFIG")
        .args(["init", "--config"])
        .assert()
        .failure();
}

#[test]
fn init_without_name_or_config_errors() {
    let dir = TempDir::new().unwrap();
    Command::cargo_bin("anvil")
        .unwrap()
        .env("RUST_LOG", "anvil=error")
        .current_dir(dir.path())
        .args(["init"])
        .assert()
        .failure();
}

// ---- anvil info: multiple versions ----

#[test]
fn info_lists_other_versions_when_multiple_exist() {
    // When several files share a name (`resolver-1.yaml`, `resolver-2.yaml`),
    // `anvil info resolver` should call out all candidate versions instead
    // of silently picking the highest.
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(
        pkg_dir.join("resolver-1.yaml"),
        "name: resolver\nversion: \"1\"\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("resolver-2.yaml"),
        "name: resolver\nversion: \"2\"\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("resolver-3.yaml"),
        "name: resolver\nversion: \"3\"\n",
    )
    .unwrap();
    let cfg_path = dir.path().join("config.yaml");
    fs::write(
        &cfg_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    anvil(cfg_path.to_str().unwrap())
        .args(["info", "resolver"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Available versions:"))
        .stdout(predicate::str::contains("1"))
        .stdout(predicate::str::contains("2"))
        .stdout(predicate::str::contains("3"));
}

// ---- verbose flag ----

#[test]
fn default_log_level_is_quiet() {
    // No RUST_LOG, no -v: info-level "Loaded N packages" should not appear.
    let (_dir, cfg_str) = setup_env();
    let mut cmd = Command::cargo_bin("anvil").unwrap();
    cmd.env("ANVIL_CONFIG", &cfg_str);
    cmd.env_remove("RUST_LOG");
    cmd.args(["list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Loaded").not());
}

#[test]
fn verbose_flag_enables_info_logs() {
    let (_dir, cfg_str) = setup_env();
    let mut cmd = Command::cargo_bin("anvil").unwrap();
    cmd.env("ANVIL_CONFIG", &cfg_str);
    cmd.env_remove("RUST_LOG");
    cmd.args(["-v", "list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Loaded"));
}

// ---- anvil shell flags ----

#[test]
fn shell_help_exposes_shim_flags() {
    let (_dir, cfg) = setup_env();
    anvil(&cfg)
        .args(["shell", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--env-only"))
        .stdout(predicate::str::contains("--no-sweep"));
}

// ---- resolver conflict diagnostics ----

/// Set up a temp dir with two python versions and two packages that pin
/// incompatible pythons.  Returns (TempDir, config_path).
fn setup_conflicting_pythons() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    for v in ["3.10", "3.11"] {
        let d = pkg_dir.join(format!("python/{}", v));
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("package.yaml"),
            format!("name: python\nversion: \"{}\"\n", v),
        )
        .unwrap();
    }

    fs::write(
        pkg_dir.join("alpha-1.0.yaml"),
        "name: alpha\nversion: \"1.0\"\nrequires:\n  - python-3.10\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("beta-1.0.yaml"),
        "name: beta\nversion: \"1.0\"\nrequires:\n  - python-3.11\n",
    )
    .unwrap();

    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    (dir, config_path.to_string_lossy().to_string())
}

#[test]
fn conflict_lists_both_requesters_and_constraints() {
    let (_dir, cfg) = setup_conflicting_pythons();
    anvil(&cfg)
        .args(["env", "alpha-1.0", "beta-1.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("version conflict for 'python'"))
        .stderr(predicate::str::contains("alpha-1.0 required python-3.10"))
        .stderr(predicate::str::contains("beta-1.0 required python-3.11"))
        .stderr(predicate::str::contains("INCOMPATIBLE"));
}

#[test]
fn missing_version_names_the_requester() {
    let (_dir, cfg) = setup_conflicting_pythons();
    // Ask for a python version that doesn't exist; the error should
    // attribute the failing constraint to the top-level request.
    anvil(&cfg)
        .args(["env", "python-3.99"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No version of 'python'"))
        .stderr(predicate::str::contains("required by <request>"))
        .stderr(predicate::str::contains("3.10"))
        .stderr(predicate::str::contains("3.11"));
}

// ---- lockfile content hashes ----

#[test]
fn lock_records_content_hashes() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    assert!(lock.contains("content_hash:"), "lockfile should record hashes:\n{}", lock);
    // SHA-256 hex digest is 64 chars; spot-check that something hex-shaped is there.
    assert!(
        lock.lines().any(|l| l.contains("content_hash:")
            && l.split(':').last().unwrap().trim().len() >= 32),
        "lockfile hash should be a long hex digest:\n{}",
        lock,
    );
}

#[test]
fn legacy_string_form_lockfile_still_parses() {
    let (dir, cfg) = setup_env();
    // Write a legacy-format lockfile by hand (pre-0.5 string-valued pins).
    fs::write(
        dir.path().join("anvil.lock"),
        "requests:\n  - maya-2024\npins:\n  maya: \"2024\"\n  python: \"3.11\"\n",
    )
    .unwrap();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["env", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MAYA_VERSION=2024"));
}

#[test]
fn drift_warning_when_package_changes_after_lock() {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    let pkg_path = pkg_dir.join("widget-1.0.yaml");
    fs::write(
        &pkg_path,
        "name: widget\nversion: \"1.0\"\nenvironment:\n  WIDGET: original\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    let cfg = config_path.to_string_lossy().to_string();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "widget-1.0"])
        .assert()
        .success();

    // Tamper: same version, different bytes.
    fs::write(
        &pkg_path,
        "name: widget\nversion: \"1.0\"\nenvironment:\n  WIDGET: TAMPERED\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("anvil").unwrap();
    cmd.env("ANVIL_CONFIG", &cfg);
    cmd.env("RUST_LOG", "anvil=warn");
    cmd.current_dir(dir.path())
        .args(["env", "widget-1.0", "--refresh"])
        .assert()
        .success()
        .stderr(predicate::str::contains("lockfile drift"))
        .stderr(predicate::str::contains("widget-1.0"));
}

// ---- anvil add / anvil remove ----

#[test]
fn add_creates_lockfile_when_none_exists() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "maya-2024"])
        .assert()
        .success();
    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    assert!(lock.contains("maya-2024"));
    assert!(lock.contains("requests:"));
}

#[test]
fn add_appends_to_existing_request_set() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "maya-2024"])
        .assert()
        .success();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "studio-blender-tools-1.0.0"])
        .assert()
        .success();
    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    assert!(lock.contains("maya-2024"), "{}", lock);
    assert!(lock.contains("studio-blender-tools-1.0.0"), "{}", lock);
}

#[test]
fn add_replaces_request_with_same_name() {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    for v in ["1.0", "2.0"] {
        fs::write(
            pkg_dir.join(format!("widget-{}.yaml", v)),
            format!("name: widget\nversion: \"{}\"\n", v),
        )
        .unwrap();
    }
    let cfg_path = dir.path().join("config.yaml");
    fs::write(&cfg_path, format!("package_paths:\n  - {}\n", pkg_dir.display())).unwrap();
    let cfg = cfg_path.to_string_lossy().to_string();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "widget-1.0"])
        .assert()
        .success();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "widget-2.0"])
        .assert()
        .success();
    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    // Both requests for widget should not coexist; the latest add wins.
    assert!(lock.contains("widget-2.0"), "{}", lock);
    assert!(!lock.contains("widget-1.0"), "old version should be replaced:\n{}", lock);
}

#[test]
fn remove_drops_requested_name() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "maya-2024"])
        .assert()
        .success();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "studio-blender-tools-1.0.0"])
        .assert()
        .success();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["remove", "studio-blender-tools"])
        .assert()
        .success();
    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    assert!(lock.contains("maya-2024"), "{}", lock);
    assert!(
        !lock.contains("studio-blender-tools"),
        "studio-blender-tools should be gone:\n{}",
        lock,
    );
}

#[test]
fn remove_refuses_to_empty_the_lockfile() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["add", "maya-2024"])
        .assert()
        .success();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["remove", "maya"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("empty lockfile"));
}

#[test]
fn remove_without_lockfile_fails() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["remove", "anything"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no anvil.lock to mutate"));
}

// ---- anvil lock --upgrade-package ----

#[test]
fn upgrade_package_only_re_resolves_named_package() {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    // Two python versions, two arnold versions.  Both packages
    // accept any version of either dep.
    for v in ["3.10", "3.11"] {
        fs::write(
            pkg_dir.join(format!("python-{}.yaml", v)),
            format!("name: python\nversion: \"{}\"\n", v),
        )
        .unwrap();
    }
    for v in ["7.1", "7.2"] {
        fs::write(
            pkg_dir.join(format!("arnold-{}.yaml", v)),
            format!("name: arnold\nversion: \"{}\"\n", v),
        )
        .unwrap();
    }
    fs::write(
        pkg_dir.join("maya-2024.yaml"),
        "name: maya\nversion: \"2024\"\nrequires:\n  - python\n  - arnold\n",
    )
    .unwrap();

    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    let cfg = config_path.to_string_lossy().to_string();

    // Initial lock — pins highest of each.
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();
    let initial = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    assert!(initial.contains("version: '3.11'"), "{}", initial);
    assert!(initial.contains("version: '7.2'"), "{}", initial);

    // Hand-edit the lock to pin python at 3.10 (simulate a project
    // that's been on 3.10 for a while).
    let edited = initial
        .replace("version: '3.11'", "version: '3.10'")
        .replace("version: '7.2'", "version: '7.1'");
    fs::write(dir.path().join("anvil.lock"), &edited).unwrap();

    // Re-lock with --upgrade-package python: python should bump to
    // 3.11, arnold should stay at the existing pin (7.1).
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024", "--upgrade-package", "python"])
        .assert()
        .success();
    let after = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    assert!(after.contains("version: '3.11'"), "python should upgrade:\n{}", after);
    assert!(after.contains("version: '7.1'"), "arnold should stay pinned:\n{}", after);
    assert!(!after.contains("version: '7.2'"), "arnold should NOT bump to 7.2:\n{}", after);
}

#[test]
fn upgrade_package_without_lockfile_fails() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024", "--upgrade-package", "python"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--upgrade-package needs an existing anvil.lock"));
}

// ---- anvil tree ----

#[test]
fn tree_renders_dependency_graph_with_connectors() {
    // Build a tree: app -> [foo, bar]; foo -> shared; bar -> shared.
    // The second occurrence of `shared` should be marked `(*)` so
    // the diamond doesn't print twice.
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    fs::write(
        pkg_dir.join("shared-1.0.yaml"),
        "name: shared\nversion: \"1.0\"\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("foo-1.0.yaml"),
        "name: foo\nversion: \"1.0\"\nrequires:\n  - shared-1.0\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("bar-1.0.yaml"),
        "name: bar\nversion: \"1.0\"\nrequires:\n  - shared-1.0\n",
    )
    .unwrap();
    fs::write(
        pkg_dir.join("app-1.0.yaml"),
        "name: app\nversion: \"1.0\"\nrequires:\n  - foo-1.0\n  - bar-1.0\n",
    )
    .unwrap();

    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();

    let assert = anvil(&config_path.to_string_lossy())
        .args(["tree", "app-1.0"])
        .assert()
        .success();
    let stdout = String::from_utf8_lossy(&assert.get_output().stdout).to_string();

    assert!(stdout.starts_with("app-1.0"), "should print root first:\n{}", stdout);
    assert!(stdout.contains("├── foo-1.0"), "non-last child uses ├──:\n{}", stdout);
    assert!(stdout.contains("└── bar-1.0"), "last child uses └──:\n{}", stdout);
    assert!(stdout.contains("shared-1.0"), "shared dep should appear:\n{}", stdout);
    assert!(stdout.contains("(*)"), "repeat marker for diamond dep:\n{}", stdout);
}

// ---- anvil sync ----

#[test]
fn sync_succeeds_when_pinned_packages_are_present() {
    // The test fixture's command targets point to placeholder paths
    // that don't exist on disk, so sync prints warnings -- but as
    // long as the pinned package definitions resolve and their
    // content hashes match, sync should still exit 0.
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("maya-2024"))
        .stdout(predicate::str::contains("python-3.11"))
        .stdout(predicate::str::contains("0 failure(s)"));
}

#[test]
fn sync_fails_when_pinned_version_missing() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    // Hand-edit the lock to a version that doesn't exist on disk.
    let lock_path = dir.path().join("anvil.lock");
    let original = fs::read_to_string(&lock_path).unwrap();
    fs::write(&lock_path, original.replace("version: '2024'", "version: '1999'")).unwrap();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["sync"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("fail  maya-1999"))
        .stderr(predicate::str::contains("anvil sync"));
}

#[test]
fn sync_warns_on_hash_drift() {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    let pkg_path = pkg_dir.join("widget-1.0.yaml");
    fs::write(
        &pkg_path,
        "name: widget\nversion: \"1.0\"\nenvironment:\n  WIDGET: original\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    let cfg = config_path.to_string_lossy().to_string();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "widget-1.0"])
        .assert()
        .success();

    fs::write(
        &pkg_path,
        "name: widget\nversion: \"1.0\"\nenvironment:\n  WIDGET: tampered\n",
    )
    .unwrap();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["sync", "--refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("warn  widget-1.0"))
        .stdout(predicate::str::contains("content hash drift"))
        .stdout(predicate::str::contains("1 warning(s)"));
}

#[test]
fn sync_fails_without_a_lockfile() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["sync"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no anvil.lock"));
}

// ---- --locked / --frozen ----

#[test]
fn locked_passes_when_lockfile_matches_disk() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["--locked", "env", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MAYA_VERSION=2024"));
}

#[test]
fn locked_fails_when_lockfile_is_stale() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    // Hand-edit the lock to a version that doesn't exist on disk.
    let lock_path = dir.path().join("anvil.lock");
    let original = fs::read_to_string(&lock_path).unwrap();
    let stale = original.replace("version: '2024'", "version: '1999'");
    fs::write(&lock_path, &stale).unwrap();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["--locked", "env", "maya-2024"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--locked: anvil.lock is stale"));
}

#[test]
fn locked_fails_without_a_lockfile() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["--locked", "env", "maya-2024"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--locked: no anvil.lock"));
}

#[test]
fn frozen_uses_lockfile_only() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["--frozen", "env", "maya-2024"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MAYA_VERSION=2024"));
}

#[test]
fn frozen_fails_for_unpinned_package() {
    let (dir, cfg) = setup_env();
    // Lock only maya — python is a transitive dep that *will* be pinned.
    // Then ask for studio-blender-tools which was never resolved/pinned.
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "maya-2024"])
        .assert()
        .success();

    anvil(&cfg)
        .current_dir(dir.path())
        .args(["--frozen", "env", "studio-blender-tools-1.0.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--frozen"))
        .stderr(predicate::str::contains("studio-blender-tools"));
}

#[test]
fn frozen_without_lockfile_fails() {
    let (dir, cfg) = setup_env();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["--frozen", "env", "maya-2024"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--frozen requires anvil.lock"));
}

// ---- cross-platform lockfile ----

/// Set up a temp dir with a package whose `variants:` block adds a
/// different transitive dep on each platform, plus the per-platform
/// candidate packages.  Returns (TempDir, config_path).
fn setup_per_platform_variants() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();

    // Three platform-specific runtimes that only one platform pulls in.
    for (name, ver) in [("gcc-runtime", "7"), ("clang-runtime", "15"), ("msvc-runtime", "2022")] {
        let d = pkg_dir.join(format!("{}/{}", name, ver));
        fs::create_dir_all(&d).unwrap();
        fs::write(
            d.join("package.yaml"),
            format!("name: {}\nversion: \"{}\"\n", name, ver),
        )
        .unwrap();
    }

    // omega-1.0 pulls in a different runtime on each platform.
    fs::write(
        pkg_dir.join("omega-1.0.yaml"),
        r#"
name: omega
version: "1.0"
variants:
  - platform: linux
    requires:
      - gcc-runtime-7
  - platform: macos
    requires:
      - clang-runtime-15
  - platform: windows
    requires:
      - msvc-runtime-2022
"#,
    )
    .unwrap();

    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    (dir, config_path.to_string_lossy().to_string())
}

#[test]
fn lock_all_platforms_records_per_platform_pins() {
    let (dir, cfg) = setup_per_platform_variants();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "omega-1.0", "--all-platforms"])
        .assert()
        .success();

    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    // omega is the same on every platform — common pin.
    assert!(lock.contains("omega"), "omega should be pinned:\n{}", lock);
    // Each runtime shows up under its platform overlay.
    assert!(
        lock.contains("platform_pins:"),
        "expected platform_pins overlay:\n{}",
        lock,
    );
    assert!(lock.contains("gcc-runtime"), "missing linux runtime:\n{}", lock);
    assert!(lock.contains("clang-runtime"), "missing macos runtime:\n{}", lock);
    assert!(lock.contains("msvc-runtime"), "missing windows runtime:\n{}", lock);
    // Lockfile records which platforms it covers.
    assert!(lock.contains("platforms:"), "missing platforms list:\n{}", lock);
}

#[test]
fn current_platform_lock_skips_overlay() {
    let (dir, cfg) = setup_per_platform_variants();
    anvil(&cfg)
        .current_dir(dir.path())
        .args(["lock", "omega-1.0"])
        .assert()
        .success();

    let lock = fs::read_to_string(dir.path().join("anvil.lock")).unwrap();
    // Without --all-platforms, only the running platform is locked.
    // The overlay should be absent (skip_serializing_if = empty).
    assert!(
        !lock.contains("platform_pins:"),
        "single-platform lock should not emit overlay:\n{}",
        lock,
    );
}

#[test]
fn missing_dep_names_the_parent_package() {
    let dir = TempDir::new().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    // alpha requires a package that doesn't exist anywhere.
    fs::write(
        pkg_dir.join("alpha-1.0.yaml"),
        "name: alpha\nversion: \"1.0\"\nrequires:\n  - missing-pkg-1.0\n",
    )
    .unwrap();
    let config_path = dir.path().join("config.yaml");
    fs::write(
        &config_path,
        format!("package_paths:\n  - {}\n", pkg_dir.display()),
    )
    .unwrap();
    anvil(&config_path.to_string_lossy())
        .args(["env", "alpha-1.0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Package not found: 'missing-pkg'"))
        .stderr(predicate::str::contains("required by alpha-1.0"));
}
