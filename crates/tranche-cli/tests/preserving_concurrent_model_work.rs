use serde_json::json;
use tranche_cli::checkpoint::Pass;

#[test]
#[ignore = "helper launched by the crash-recovery regression"]
fn checkpoint_lock_holder() {
    let Ok(root) = std::env::var("TRANCHE_TEST_LOCK_ROOT") else {
        return;
    };
    let root = std::path::Path::new(&root);
    let pass = Pass::acquire(&root.join("pairs.jsonl")).unwrap();
    let checkpoint = pass.checkpoint(false).unwrap();
    checkpoint.append(0, &json!({"pair": [1, 2]})).unwrap();
    std::fs::write(root.join("ready"), b"ready").unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[test]
fn overlapping_passes_are_refused_without_erasing_synced_work() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("judgments.jsonl");
    let pass = Pass::acquire(&path).unwrap();
    let checkpoint = pass.checkpoint(false).unwrap();
    checkpoint.append(0, &json!({"number": 1})).unwrap();
    assert!(Pass::acquire(&path).is_err());
    checkpoint.finish(true).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"number\":1}\n");
    drop(checkpoint);
    drop(pass);
    let resumed = Pass::acquire(&path).unwrap();
    resumed.recover().unwrap();
}

#[test]
fn a_crashed_writer_releases_the_lock_and_preserves_its_journal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("pairs.jsonl");
    let mut writer = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "checkpoint_lock_holder"])
        .env("TRANCHE_TEST_LOCK_ROOT", root.path())
        .spawn()
        .unwrap();
    let ready = root.path().join("ready");
    let started = std::time::Instant::now();
    while !ready.exists() && started.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let contention = ready.exists() && Pass::acquire(&path).is_err();
    writer.kill().unwrap();
    writer.wait().unwrap();
    assert!(contention, "another process must hold the model-pass lock");
    let resumed = Pass::acquire(&path).unwrap();
    resumed.recover().unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "{\"pair\":[1,2]}\n"
    );
}
