//! The GitHub transport's contract, with a fake `gh` in a directory.
//!
//! The property that matters is the refusal: a failure or an unexpected body must
//! never read as "no more PRs", because that would silently truncate the corpus
//! and every report built from it would be short without saying so.
//!
//! The fake command lives in its own directory and is passed in, so no test
//! changes what a test beside it sees.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use std::time::{Duration, Instant};
use tranche_core::gh::{GitHubError, Transport, page_with, page_with_timeout};

const URL: &str =
    "https://api.github.com/repos/omacom/omarchy/pulls?state=open&per_page=100&page=1";

/// A directory holding one fake command that prints the given streams and exits
/// with the given status.
fn fake(name: &str, stdout: &str, stderr: &str, status: i32) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let binary: PathBuf = directory.path().join(name);
    // `printf` rather than a heredoc: a heredoc appends a newline and needs care
    // with quoting inside JSON.
    let escape = |text: &str| text.replace('\\', "\\\\").replace('\'', "'\\''");
    let script = format!(
        "#!/bin/sh\nprintf '%s' '{}'\nprintf '%s' '{}' >&2\nexit {status}\n",
        escape(stdout),
        escape(stderr)
    );
    install_script(&binary, &script);
    directory
}

fn install_script(binary: &Path, script: &str) {
    fs::write(binary, script).expect("write the fake");
    fs::set_permissions(binary, fs::Permissions::from_mode(0o755)).expect("make it runnable");
    // Make sure the inode is fully written and the handle released before the
    // file is executed, or the kernel reports "text file busy".
    let handle = fs::OpenOptions::new()
        .read(true)
        .open(binary)
        .expect("reopen the fake");
    handle.sync_all().expect("flush the fake");
    drop(handle);
}

fn read(directory: &tempfile::TempDir) -> Result<Vec<serde_json::Value>, GitHubError> {
    page_with(Transport::Gh, URL, Some(Path::new(directory.path())))
}

#[test]
fn a_page_is_the_json_array_the_api_returned() {
    let path = fake("gh", r#"[{"number": 1}, {"number": 2}]"#, "", 0);
    let items = read(&path).expect("a page comes back");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["number"], 1);
}

#[test]
fn a_valid_page_larger_than_pipe_capacity_finishes() {
    let title = "x".repeat(256 * 1024);
    let body = serde_json::json!([{ "number": 9, "title": title }]).to_string();
    let path = fake("gh", &body, "", 0);
    let items = read(&path).expect("a large valid page must not time out behind a full pipe");
    assert_eq!(items[0]["title"], title);
}

#[test]
fn an_empty_array_is_an_empty_page_not_an_error() {
    let path = fake("gh", "[]", "", 0);
    assert!(read(&path).expect("an empty page").is_empty());
}

#[test]
fn a_failed_command_reports_and_does_not_look_like_an_empty_page() {
    // The whole point: a failure must not be read as "no more PRs".
    let path = fake("gh", "", "HTTP 403: rate limit exceeded", 1);
    let error = read(&path).expect_err("a failure is fatal");
    let message = error.to_string();
    assert!(message.contains("rate limit exceeded"), "{message}");
    assert!(
        message.contains("previous snapshot is retained"),
        "{message}"
    );
}

#[test]
fn a_non_array_body_is_refused() {
    // An error document returned with a success status would otherwise read as a
    // page with no PRs.
    let path = fake("gh", r#"{"message": "Not Found"}"#, "", 0);
    let error = read(&path).expect_err("a non-array is fatal");
    assert!(
        error.to_string().contains("did not return a PR list"),
        "{error}"
    );
}

#[test]
fn a_body_that_is_not_json_is_refused() {
    let path = fake("gh", "<html>gateway timeout</html>", "", 0);
    let error = read(&path).expect_err("unreadable JSON is fatal");
    assert!(error.to_string().contains("unreadable JSON"), "{error}");
}

#[test]
fn large_stderr_is_drained_and_the_last_error_is_retained() {
    let stderr = format!(
        "{}\nHTTP 403: final cause\n",
        "progress\n".repeat(32 * 1024)
    );
    let path = fake("gh", "", &stderr, 1);
    let error = read(&path).expect_err("the exit status remains a refusal");
    assert!(
        error.to_string().contains("HTTP 403: final cause"),
        "{error}"
    );
    assert!(error.to_string().contains("previous snapshot is retained"));
}

#[test]
fn both_large_streams_are_drained_on_success() {
    let title = "x".repeat(256 * 1024);
    let body = serde_json::json!([{ "title": title }]).to_string();
    let path = fake("gh", &body, &"warning\n".repeat(32 * 1024), 0);
    assert_eq!(read(&path).expect("both pipes drained")[0]["title"], title);
}

#[test]
fn oversized_stdout_is_refused_without_touching_the_snapshot() {
    let path = fake("gh", &" ".repeat(32 * 1024 * 1024 + 1), "", 0);
    let snapshot = path.path().join("snapshot.json");
    fs::write(&snapshot, b"previous observation").unwrap();
    let started = Instant::now();
    let error = read(&path).expect_err("stdout must be bounded");
    assert!(
        error.to_string().contains("stdout exceeded output limit"),
        "{error}"
    );
    assert!(error.to_string().contains("previous snapshot is retained"));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fs::read(snapshot).unwrap(), b"previous observation");
}

#[test]
fn oversized_stderr_is_refused() {
    let path = fake("gh", "[]", &"x".repeat(1024 * 1024 + 1), 0);
    let error = read(&path).expect_err("stderr must be bounded");
    assert!(
        error.to_string().contains("stderr exceeded output limit"),
        "{error}"
    );
    assert!(error.to_string().contains("previous snapshot is retained"));
}

#[test]
fn a_timeout_kills_and_reaps_the_command() {
    let path = fake("gh", "", "", 0);
    let pid_file = path.path().join("pid");
    install_script(
        &path.path().join("gh"),
        &format!(
            "#!/bin/sh\necho $$ > '{}'\nexec sleep 60\n",
            pid_file.display()
        ),
    );
    let started = Instant::now();
    let error = page_with_timeout(
        Transport::Gh,
        URL,
        Some(path.path()),
        Duration::from_millis(200),
    )
    .expect_err("a hung command must time out");
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(error.to_string().contains("previous snapshot is retained"));
    assert!(started.elapsed() < Duration::from_secs(2));
    let pid: libc::pid_t = fs::read_to_string(pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "command still alive");
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
    assert_eq!(
        unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) },
        -1
    );
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ECHILD)
    );
}

#[test]
fn inherited_pipes_do_not_outlive_the_deadline() {
    let path = fake("gh", "", "", 0);
    install_script(
        &path.path().join("gh"),
        "#!/bin/sh\nsleep 60 &\nprintf '[]'\nexit 0\n",
    );
    let started = Instant::now();
    let error = page_with_timeout(
        Transport::Gh,
        URL,
        Some(path.path()),
        Duration::from_millis(200),
    )
    .expect_err("an inherited writer must not hang reader cleanup");
    assert!(error.to_string().contains("timed out"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn curl_is_a_separate_transport_with_the_same_contract() {
    let path = fake("curl", r#"[{"number": 7}]"#, "", 0);
    let items = page_with(Transport::Curl, URL, Some(Path::new(path.path())))
        .expect("curl reads the same shape");
    assert_eq!(items[0]["number"], 7);
}
