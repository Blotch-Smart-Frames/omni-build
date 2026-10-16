//! The `omni-build` command line.

use clap::Parser;

/// Build, sign, install and ship iOS and Android apps with no native toolchain.
#[derive(Debug, Parser)]
#[command(version, about, arg_required_else_help = true)]
struct Cli {}

fn main() {
    // No commands yet: the Phase 0 spikes in #1 add them. Parsing already
    // answers --help and --version, and rejects unknown arguments.
    Cli::parse();
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::Cli;

    #[test]
    fn cli_definition_is_valid() {
        // clap checks a command's definition (conflicting flags, duplicate
        // names) only when it's built, so a mistake would otherwise surface
        // as a panic the first time a user runs that command.
        Cli::command().debug_assert();
    }
}
