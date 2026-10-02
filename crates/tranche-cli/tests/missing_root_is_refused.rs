use std::path::Path;
use std::process::{Command, Output};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tranche"))
        .arg("--root")
        .arg(root)
        .args(args)
        .output()
        .expect("the tranche binary should be runnable")
}

fn temp_root() -> tempfile::TempDir {
    tempfile::tempdir().expect("a temporary root")
}

#[test]
fn a_root_without_reports_is_refused_without_a_traceback() {
    let root = temp_root();
    let output = run(root.path(), &["cluster"]);
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    // A refusal an operator can act on, not a panic dump.
    assert!(!stderr.contains("panicked"), "{stderr}");
    assert!(!stderr.contains("Traceback"), "{stderr}");
}

#[test]
fn a_bare_invocation_prints_the_command_tree() {
    let root = temp_root();
    for args in [vec![], vec!["evidence"]] {
        let output = run(root.path(), &args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        // clap answers a bare invocation with help on stderr and a usage status.
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Usage:"), "{args:?}: {stderr}");
        assert!(stderr.contains("evidence"), "{args:?}: {stderr}");
    }
}
