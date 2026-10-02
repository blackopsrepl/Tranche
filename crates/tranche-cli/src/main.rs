use clap::Parser;
use tranche_cli::{cli::Cli, commands};

fn main() {
    let cli = Cli::parse();
    let outcome = commands::run(&cli);
    eprint!("{}", outcome.stderr);
    print!("{}", outcome.stdout);
    std::process::exit(outcome.code);
}
