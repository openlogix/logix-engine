//! Command-line tests (docs/command-line.md): arguments that make the
//! engine exit before starting MCP.

use std::process::{Command, Output};

const ENGINE: &str = env!("CARGO_BIN_EXE_logix-engine");

fn run(args: &[&str]) -> Output {
    Command::new(ENGINE)
        .args(args)
        .output()
        .expect("run the engine")
}

#[test]
fn version_prints_the_date_version_and_exits() {
    let out = run(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    let version = stdout.trim().strip_prefix("logix-engine ").expect(&stdout);
    assert!(
        version.len() >= 10 && &version[4..5] == "-" && &version[7..8] == "-",
        "{version}"
    );
}

#[test]
fn help_lists_the_arguments_and_exits() {
    let out = run(&["--help"]);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    for arg in [
        "--engine-folder",
        "--no-auto-install",
        "--log-level",
        "--version",
        "--help",
    ] {
        assert!(stdout.contains(arg), "{arg} missing from:\n{stdout}");
    }
}

fn assert_fails_on_stderr(out: Output, expected: &str) {
    assert!(!out.status.success());
    assert!(out.stdout.is_empty(), "stdout is only for MCP");
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains(expected),
        "expected {expected:?} in:\n{stderr}"
    );
}

#[test]
fn engine_folder_is_required() {
    assert_fails_on_stderr(run(&[]), "--engine-folder");
}

#[test]
fn unknown_arguments_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("engine");
    assert_fails_on_stderr(
        run(&["--engine-folder", folder.to_str().unwrap(), "--bogus"]),
        "--bogus",
    );
    assert_fails_on_stderr(
        run(&[
            "--engine-folder",
            folder.to_str().unwrap(),
            "--log-level",
            "loud",
        ]),
        "loud",
    );
}

#[test]
fn an_engine_folder_that_fails_the_checks_is_refused_and_not_created() {
    let tmp = tempfile::tempdir().unwrap();
    let folder = tmp.path().join("tools");
    assert_fails_on_stderr(
        run(&["--engine-folder", folder.to_str().unwrap()]),
        "must contain \"engine\"",
    );
    assert!(!folder.exists());

    assert_fails_on_stderr(run(&["--engine-folder", "engine"]), "not an absolute path");
    assert_fails_on_stderr(
        run(&[
            "--engine-folder",
            &format!("{}/x/../engine", tmp.path().display()),
        ]),
        "'..'",
    );
}

#[test]
fn the_home_folder_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("engine");
    std::fs::create_dir(&home).unwrap();
    let out = Command::new(ENGINE)
        .args(["--engine-folder", home.to_str().unwrap()])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert_fails_on_stderr(out, "home folder");
}
