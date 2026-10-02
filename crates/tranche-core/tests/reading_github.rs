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

use tranche_core::gh::{GitHubError, Transport, page_with};

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
    fs::write(&binary, script).expect("write the fake");
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).expect("make it runnable");
    // Make sure the inode is fully written and the handle released before the
    // file is executed, or the kernel reports "text file busy".
    let handle = fs::OpenOptions::new()
        .read(true)
        .open(&binary)
        .expect("reopen the fake");
    handle.sync_all().expect("flush the fake");
    drop(handle);
    directory
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
fn curl_is_a_separate_transport_with_the_same_contract() {
    let path = fake("curl", r#"[{"number": 7}]"#, "", 0);
    let items = page_with(Transport::Curl, URL, Some(Path::new(path.path())))
        .expect("curl reads the same shape");
    assert_eq!(items[0]["number"], 7);
}
