use assert_cmd::Command;

fn llmux() -> Command {
    Command::cargo_bin("llmux").unwrap()
}

#[test]
fn alias_s_for_spawn() {
    // "s --help" should behave like "spawn --help"
    let out = llmux().args(["s", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Spawn a new agent session"),
        "expected spawn help text, got:\n{stdout}"
    );
}

#[test]
fn alias_l_for_ls() {
    let out = llmux().args(["l", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("List running sessions"),
        "expected ls help text, got:\n{stdout}"
    );
}

#[test]
fn alias_a_for_attach() {
    let out = llmux().args(["a", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Attach to a running session"),
        "expected attach help text, got:\n{stdout}"
    );
}

#[test]
fn alias_uppercase_h_for_history() {
    let out = llmux().args(["H", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Show session history"),
        "expected history help text, got:\n{stdout}"
    );
}

#[test]
fn alias_r_for_resume() {
    let out = llmux().args(["r", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Resume a previous session"),
        "expected resume help text, got:\n{stdout}"
    );
}

#[test]
fn alias_k_for_kill() {
    let out = llmux().args(["k", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Kill a running session"),
        "expected kill help text, got:\n{stdout}"
    );
}

#[test]
fn alias_c_for_clean() {
    let out = llmux().args(["c", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Clean up stale sessions"),
        "expected clean help text, got:\n{stdout}"
    );
}

#[test]
fn config_has_no_single_letter_alias() {
    // "c" should resolve to clean, not config
    let out = llmux().args(["c", "--help"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("Show configuration"),
        "'c' should not resolve to config"
    );
}
