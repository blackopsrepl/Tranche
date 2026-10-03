use std::process::Command;

#[test]
fn malformed_init_never_mutates_an_existing_contract_even_with_force() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tranche.json");
    std::fs::write(&path, b"original bytes\n").unwrap();
    for repository in ["a/b/c", "a/b?x", "a/..", "a/b c"] {
        let output = Command::new(env!("CARGO_BIN_EXE_tranche"))
            .args(["init", repository, "--force", "--root"])
            .arg(root.path())
            .output()
            .unwrap();
        assert!(!output.status.success(), "{repository}");
        assert_eq!(std::fs::read(&path).unwrap(), b"original bytes\n");
    }
}

#[test]
fn init_preserves_existing_contract_without_force() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("tranche.json");
    std::fs::write(&path, b"original bytes\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_tranche"))
        .args(["init", "a/b", "--root"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), b"original bytes\n");
}
