use clap::Parser;

/// Local hired-driver history and analytics for Euro Truck Simulator 2.
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli;

fn main() {
    Cli::parse();
}
