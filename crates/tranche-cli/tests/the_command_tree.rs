use clap::{CommandFactory, Parser};
use tranche_cli::cli::{Cli, Command, Evidence, Transport};

fn parse(args: &[&str]) -> Cli {
    match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => panic!("{args:?} should parse: {error}"),
    }
}

#[test]
fn every_documented_invocation_parses() {
    Cli::command().debug_assert();
    let cases: &[&[&str]] = &[
        &["fetch", "--transport", "urllib"],
        &["judge", "--limit", "2", "--resume"],
        &["dupes", "--max-pairs", "0"],
        &["cluster", "--allow-unbound"],
        &["batches"],
        &["refresh", "--max-pairs", "12", "--no-page", "--dry-run"],
        &["all", "--limit", "1", "--resume", "--max-pairs", "0"],
        &["page"],
        &["evidence", "capture", "--batch", "B001"],
        &["evidence", "show", "--capture", "C-123"],
        &["evidence", "export", "--capture", "C-123", "--output", "-"],
    ];
    for args in cases {
        let mut argv = vec!["tranche"];
        argv.extend_from_slice(args);
        argv.extend_from_slice(&["--json", "--root", "."]);
        assert!(parse(&argv).json, "{args:?}");
    }
}

#[test]
fn invalid_values_are_refused() {
    for args in [
        ["tranche", "judge", "--limit", "0"],
        ["tranche", "dupes", "--max-pairs", "-1"],
        ["tranche", "fetch", "--transport", "nonsense"],
    ] {
        // A typo or an out-of-range value must be rejected at parse time rather
        // than reaching an operation that cannot honour it.
        assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
    }
}

#[test]
fn mutually_exclusive_selectors_are_refused() {
    // `--batch` (current association) and `--capture` (historical) are
    // different questions; asking both is a usage error, not a precedence rule.
    for args in [
        &[
            "tranche",
            "evidence",
            "show",
            "--batch",
            "B001",
            "--capture",
            "C-123",
        ][..],
        &[
            "tranche",
            "evidence",
            "show",
            "--capture",
            "C-123",
            "--source",
            "S1",
            "--citation",
            "Q1",
        ][..],
    ] {
        assert!(Cli::try_parse_from(args).is_err(), "{args:?}");
    }
}

#[test]
fn evidence_capture_flags_are_typed() {
    let cli = parse(&[
        "tranche",
        "evidence",
        "capture",
        "--batch",
        "B001",
        "--request-budget",
        "50",
        "--max-bytes",
        "999",
        "--fresh",
        "--reuse-capture",
        "C-123",
        "--break-lock",
    ]);
    let Command::Evidence { command } = cli.command else {
        panic!("expected an evidence command");
    };
    let Evidence::Capture(capture) = command else {
        panic!("expected an evidence capture command");
    };
    assert_eq!(capture.batch, "B001");
    assert_eq!(capture.request_budget, 50);
    assert_eq!(capture.max_bytes, 999);
    assert!(capture.fresh);
    assert_eq!(capture.reuse_capture.as_deref(), Some("C-123"));
    assert!(capture.break_lock);
}

#[test]
fn evidence_show_flags_are_typed() {
    let cli = parse(&[
        "tranche",
        "evidence",
        "show",
        "--capture",
        "C-123",
        "--source",
        "S1",
        "--start-byte",
        "4",
        "--length",
        "9",
    ]);
    let Command::Evidence { command } = cli.command else {
        panic!("expected an evidence command");
    };
    let Evidence::Show(show) = command else {
        panic!("expected an evidence show command");
    };
    assert_eq!(show.capture.as_deref(), Some("C-123"));
    assert_eq!(show.source.as_deref(), Some("S1"));
    assert_eq!(show.start_byte, 4);
    assert_eq!(show.length, 9);
}

#[test]
fn the_transport_set_is_closed() {
    let cli = parse(&["tranche", "fetch", "--transport", "gh"]);
    let Command::Fetch(fetch) = cli.command else {
        panic!("expected a fetch command");
    };
    assert_eq!(fetch.transport, Transport::Gh);
}

#[test]
fn a_bare_invocation_is_a_usage_error() {
    // Asking for nothing is answered with help, not with an operation; clap
    // reports it as a display-help error and the binary exits non-zero.
    for args in [
        vec!["tranche"],
        vec!["tranche", "evidence"],
        vec!["tranche", "--root", "."],
    ] {
        let error = Cli::try_parse_from(&args).expect_err("should ask for help");
        // `arg_required_else_help` and `subcommand_required` can each produce
        // the help-with-usage outcome; both exit 2.
        assert!(
            matches!(
                error.kind(),
                clap::error::ErrorKind::MissingSubcommand
                    | clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
            ),
            "{args:?} produced {:?}",
            error.kind()
        );
        assert_eq!(error.exit_code(), 2, "{args:?}");
    }
}
