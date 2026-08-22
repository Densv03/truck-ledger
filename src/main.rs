use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    author,
    version,
    about = "Local hired-driver history for Euro Truck Simulator 2"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Ingest {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        database: Option<PathBuf>,
    },
}
fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Ingest {
            profile,
            input,
            database,
        } => {
            let database = match database {
                Some(p) => p,
                None => match truck_ledger::default_database_path() {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("error: {e}");
                        std::process::exit(1)
                    }
                },
            };
            match truck_ledger::ingest_path(&database, &profile, &input) {
                Ok(r) => println!(
                    "hired drivers scanned: {}\nvisible trips scanned: {}\nnewly inserted trips: {}",
                    r.hired_drivers_scanned, r.visible_trips_scanned, r.newly_inserted_trips
                ),
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1)
                }
            }
        }
    }
}
