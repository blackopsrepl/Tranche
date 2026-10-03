use serde_json::json;
use tranche_core::domain::pr::load_prs;
use tranche_core::report::Root;

#[test]
fn foreign_legacy_identity_is_not_projected_into_the_deployment() {
    for (pointer, foreign) in [
        ("base", json!({"repo": {"full_name": "other/repo"}})),
        ("html_url", json!("https://github.com/other/repo/pull/1")),
        (
            "url",
            json!("https://api.github.com/repos/other/repo/pulls/1"),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let root = Root::new(temp.path());
        std::fs::create_dir_all(root.pages_dir()).unwrap();
        let mut item = json!({"number": 1, "title": "overlapping number", "body": ""});
        item[pointer] = foreign;
        std::fs::write(
            root.pages_dir().join("page_1.json"),
            json!([item]).to_string(),
        )
        .unwrap();
        assert!(load_prs(&root, "target/repo").is_err(), "{pointer}");
    }
}

#[test]
fn same_repository_shards_remain_readable_but_unidentified_membership_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let root = Root::new(temp.path());
    std::fs::create_dir_all(root.pages_dir()).unwrap();
    let items = json!([
        {"number": 1, "title": "real", "base": {"repo": {"full_name": "target/repo"}}, "html_url": "https://github.com/target/repo/pull/1"}
    ]);
    std::fs::write(root.pages_dir().join("page_1.json"), items.to_string()).unwrap();
    assert_eq!(load_prs(&root, "target/repo").unwrap().numbers(), &[1]);
    std::fs::write(
        root.pages_dir().join("page_2.json"),
        json!([{"number":2,"title":"unidentified"}]).to_string(),
    )
    .unwrap();
    assert!(load_prs(&root, "target/repo").is_err());
}
