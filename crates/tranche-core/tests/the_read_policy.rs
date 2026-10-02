//! The read transport's URL policy.
//!
//! Every check here is a refusal, which is the point: the transport runs with
//! `gh`'s credential behind it, so a URL it follows is a URL that credential is
//! spent on. A lookalike origin, a fragment, a credential in the query or a hop
//! outside the repository must all stop before a request is made.

use tranche_core::evidence::read::url;

const REPO: &str = "omacom/omarchy";

#[test]
fn the_api_origin_and_a_repository_path_are_accepted() {
    assert_eq!(
        url::validate(
            "https://api.github.com/repos/omacom/omarchy/pulls/123",
            Some(REPO)
        )
        .expect("accepted"),
        "https://api.github.com/repos/omacom/omarchy/pulls/123"
    );
    // The namespace itself, without a trailing path, is inside the scope.
    assert!(url::validate("https://api.github.com/repos/omacom/omarchy", Some(REPO)).is_ok());
}

#[test]
fn a_lookalike_origin_is_refused() {
    // Parsing alone accepts both of these; the origin check must not.
    for host in [
        "https://api.github.com.evil.test/repos/omacom/omarchy",
        "https://api.github.com@evil.test/repos/omacom/omarchy",
    ] {
        assert!(url::validate(host, Some(REPO)).is_err(), "refused: {host}");
    }
}

#[test]
fn a_non_https_scheme_is_refused() {
    assert!(url::validate("http://api.github.com/repos/omacom/omarchy", Some(REPO)).is_err());
    assert!(url::validate("file:///etc/passwd", Some(REPO)).is_err());
    assert!(url::validate("git://api.github.com/x", Some(REPO)).is_err());
}

#[test]
fn a_credential_shaped_query_parameter_is_refused() {
    // A URL is the one part of a request that gets logged everywhere.
    for parameter in [
        "?access_token=abc",
        "?TOKEN=abc",
        "?api_key=abc",
        "?client_secret=abc",
        "?Authorization=abc",
    ] {
        let url = format!("https://api.github.com/repos/omacom/omarchy/pulls{parameter}");
        assert!(url::validate(&url, Some(REPO)).is_err(), "refused: {url}");
    }
    // An ordinary parameter is not a credential and stays allowed.
    assert!(
        url::validate(
            "https://api.github.com/repos/omacom/omarchy/pulls?page=2",
            Some(REPO)
        )
        .is_ok()
    );
}

#[test]
fn whitespace_a_fragment_and_an_empty_url_are_refused() {
    assert!(url::validate("", Some(REPO)).is_err());
    assert!(
        url::validate(
            "https://api.github.com/repos/omacom/omarchy/pulls\nX-Injected: 1",
            Some(REPO)
        )
        .is_err(),
        "a newline would be a header injection"
    );
    assert!(
        url::validate(
            "https://api.github.com/repos/omacom/omarchy#frag",
            Some(REPO)
        )
        .is_err()
    );
}

#[test]
fn a_hop_outside_the_repository_namespace_is_refused() {
    // The scope check is what stops a capture from wandering into another
    // repository's data on the same token.
    assert!(
        url::validate(
            "https://api.github.com/repos/other/project/pulls/1",
            Some(REPO)
        )
        .is_err()
    );
    // A prefix that only resembles the namespace is also outside it.
    assert!(
        url::validate(
            "https://api.github.com/repos/omacom/omarchy-evil/pulls/1",
            Some(REPO)
        )
        .is_err()
    );
    // Without a scope, the origin is the only restriction.
    assert!(url::validate("https://api.github.com/repos/other/project/pulls/1", None).is_ok());
}

#[test]
fn the_continuation_link_selects_next_and_validates_it() {
    let header = "<https://api.github.com/repos/omacom/omarchy/pulls?page=2>; rel=\"next\", \
                  <https://api.github.com/repos/omacom/omarchy/pulls?page=9>; rel=\"last\"";
    assert_eq!(
        url::next_link(Some(header)).expect("parsed"),
        Some("https://api.github.com/repos/omacom/omarchy/pulls?page=2".to_owned())
    );
    // No continuation is not an error.
    assert_eq!(url::next_link(None).expect("parsed"), None);
    assert_eq!(
        url::next_link(Some("<https://api.github.com/x>; rel=\"last\"")).expect("parsed"),
        None
    );
    // A continuation pointing off-origin is refused rather than followed.
    assert!(url::next_link(Some("<https://evil.test/pulls?page=2>; rel=\"next\"")).is_err());
}

#[test]
fn the_page_number_is_read_or_defaults_to_one() {
    assert_eq!(
        url::page_number("https://api.github.com/repos/omacom/omarchy/pulls?page=7").expect("read"),
        7
    );
    assert_eq!(
        url::page_number("https://api.github.com/repos/omacom/omarchy/pulls").expect("default"),
        1
    );
    assert!(
        url::page_number("https://api.github.com/repos/omacom/omarchy/pulls?page=abc").is_err()
    );
}

#[test]
fn a_redirect_target_is_resolved_or_refused() {
    assert_eq!(
        url::absolute(
            "/repos/omacom/omarchy/pulls?page=2",
            "https://api.github.com/x"
        )
        .expect("resolved"),
        "https://api.github.com/repos/omacom/omarchy/pulls?page=2"
    );
    assert_eq!(
        url::absolute("https://api.github.com/y", "https://api.github.com/x").expect("resolved"),
        "https://api.github.com/y"
    );
    // A relative target without a leading slash cannot be resolved safely.
    assert!(url::absolute("pulls?page=2", "https://api.github.com/x").is_err());
}
