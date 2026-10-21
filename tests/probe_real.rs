//! The real `CommandRunner` exercised against a shell-script fixture —
//! covers `std::process::Command` spawn, capture, stderr fallback, and
//! the timeout kill path.

use handl_plugin_sdkgen_cli::CommandRunner;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

/// Write an executable fixture printing `body` on stdout or stderr.
fn fixture_tool(dir: &std::path::Path, name: &str, body: &str, stderr: bool) -> String {
    let path = dir.join(name);
    let redir = if stderr { " >&2" } else { "" };
    std::fs::write(&path, format!("#!/bin/sh\nprintf '%s' '{body}'{redir}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // exec of a just-written file can race the filesystem's write
    // release (ETXTBSY) — a brief settle keeps the tests deterministic.
    std::thread::sleep(Duration::from_millis(20));
    path.to_string_lossy().into_owned()
}

#[test]
fn captures_stdout_help() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(
        tmp.path(),
        "faketool",
        "Usage: faketool <arg>\\n\\nFlags:\\n  -x, --xray   do x\\n",
        false,
    );
    let out = CommandRunner::new()
        .run(&tool, &["--help".to_string()])
        .unwrap();
    assert!(out.contains("Usage: faketool"), "{out}");
    assert!(out.contains("--xray"), "{out}");
}

#[test]
fn falls_back_to_stderr_when_stdout_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = fixture_tool(tmp.path(), "errtool", "Usage: errtool\\n", true);
    let out = CommandRunner::new()
        .run(&tool, &["--help".to_string()])
        .unwrap();
    assert!(out.contains("Usage: errtool"), "{out}");
}

#[test]
fn spawn_failure_is_an_error() {
    let e = CommandRunner::new()
        .run("definitely-not-a-real-binary-xyz", &["--help".to_string()])
        .unwrap_err();
    assert!(e.contains("spawn"), "{e}");
}

#[test]
fn overlong_children_are_killed_at_the_timeout() {
    let e = CommandRunner::with_timeout(Duration::from_millis(80))
        .run("sleep", &["30".to_string()])
        .unwrap_err();
    assert!(e.contains("timed out"), "{e}");
}
