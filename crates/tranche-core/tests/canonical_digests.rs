//! The canonical encoding and the digests over it.
//!
//! Every value here is pinned from the committed corpus. These are not
//! round-trip tests: they fix the exact bytes, because a mismatch silently
//! invalidates every stored judgment and makes every report on disk foreign.

use serde_json::json;
use tranche_core::util::{KeyOrder, canonical_json, canonical_json_with, digest};

#[test]
fn canonical_encoding_matches_spaced_json_dumps() {
    // sort_keys=True, separators=(",", ":"), ensure_ascii=False
    assert_eq!(canonical_json(&json!({})), "{}");
    assert_eq!(canonical_json(&json!({"b": 1, "a": 2})), r#"{"a":2,"b":1}"#);
    assert_eq!(canonical_json(&json!([1, 2, 3])), "[1,2,3]");
    assert_eq!(canonical_json(&json!(null)), "null");
    assert_eq!(canonical_json(&json!(true)), "true");
    assert_eq!(canonical_json(&json!("hi")), r#""hi""#);
    // ensure_ascii=False keeps non-ASCII literal.
    assert_eq!(canonical_json(&json!("café")), "\"café\"");
    // Nested objects sort at every level, independent of insertion order.
    assert_eq!(
        canonical_json(&json!({"z": {"b": 1, "a": [{"d": 1, "c": 2}]}})),
        r#"{"z":{"a":[{"c":2,"d":1}],"b":1}}"#
    );
    // Floats keep the shortest round-tripping form for the values the corpus stores.
    assert_eq!(canonical_json(&json!({"score": 1.1})), r#"{"score":1.1}"#);
    assert_eq!(canonical_json(&json!({"p": 0.0})), r#"{"p":0.0}"#);
    assert_eq!(canonical_json(&json!({"p": 0.95})), r#"{"p":0.95}"#);
}

#[test]
fn digest_is_sha256_of_the_canonical_encoding() {
    // sha256('{}'), sha256('{"a":2,"b":1}')
    assert_eq!(
        digest(&json!({})),
        "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a"
    );
    assert_eq!(
        digest(&json!({"b": 1, "a": 2})),
        "d3626ac30a87e6f7a6428233b3c68299976865fa5508e4267c5415c76af7a772"
    );
    // Key order in the input must not change the digest.
    assert_eq!(
        digest(&json!({"a": 1, "b": 2})),
        digest(&json!({"b": 2, "a": 1}))
    );
}

#[test]
fn key_order_is_chosen_by_the_caller_because_key_types_differ() {
    // Keys are compared as *objects*, so the order depends on
    // the type the caller used. The report binding is keyed by PR number as an
    // integer, so `3507` precedes `10002`. JSON has only string keys, so the two
    // cases are indistinguishable from the value alone and the caller must say
    // which one applies — getting it wrong silently changes a digest.
    let value = json!({"10002": 1, "3507": 2, "999": 3});
    assert_eq!(
        canonical_json_with(&value, KeyOrder::Numeric),
        r#"{"999":3,"3507":2,"10002":1}"#
    );
    // The capture generation's `revision` map is keyed by `str(number)`, so it
    // sorts lexicographically; the default is therefore the string order.
    assert_eq!(canonical_json(&value), r#"{"10002":1,"3507":2,"999":3}"#);
    // Non-integer keys are unaffected either way, which is why the string-keyed
    // maps in the format do not care.
    assert_eq!(
        canonical_json(&json!({"b": 1, "a": 2, "C": 3})),
        r#"{"C":3,"a":2,"b":1}"#
    );
    assert_eq!(
        canonical_json_with(&json!({"b": 1, "a": 2, "C": 3}), KeyOrder::Numeric),
        r#"{"C":3,"a":2,"b":1}"#
    );
}
