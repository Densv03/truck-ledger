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
    Discover {
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
        #[arg(long)]
        database: Option<PathBuf>,
    },
    Watch {
        #[arg(long)]
        profile: String,
        #[arg(long)]
        profile_root: Option<PathBuf>,
        #[arg(long)]
        database: Option<PathBuf>,
    },
}

fn database_path(database: Option<PathBuf>) -> Result<PathBuf, truck_ledger::Error> {
    database
        .map(Ok)
        .unwrap_or_else(truck_ledger::default_database_path)
}
fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Ingest {
            profile,
            input,
            database,
        } => {
            let database = match database_path(database) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1)
                }
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
        Command::Discover { roots, database: _ } => match truck_ledger::discover(&roots) {
            Ok(candidates) => {
                if candidates.is_empty() {
                    println!("no ETS2 profile candidates found");
                }
                for (index, candidate) in candidates.iter().enumerate() {
                    println!(
                        "{}: layout={}\n   profile_root={}\n   save_root={}",
                        index + 1,
                        candidate.layout_kind,
                        candidate.profile_root.display(),
                        candidate.save_root.display()
                    );
                }
            }
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1)
            }
        },
        Command::Watch {
            profile,
            profile_root,
            database,
        } => {
            let database = database_path(database).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1)
            });
            let mut db = truck_ledger::open_database(&database).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1)
            });
            let location = match profile_root {
                Some(path) => {
                    let location = truck_ledger::validate_profile_root(&path).unwrap_or_else(|e| {
                        eprintln!("error: {e}");
                        std::process::exit(1)
                    });
                    truck_ledger::associate_profile_location(&mut db, &profile, &location)
                        .unwrap_or_else(|e| {
                            eprintln!("error: {e}");
                            std::process::exit(1)
                        });
                    location
                }
                None => truck_ledger::load_profile_location(&db, &profile).unwrap_or_else(|e| {
                    eprintln!("error: {e}");
                    std::process::exit(1)
                }),
            };
            if let Err(e) = truck_ledger::watch(&mut db, &profile, &location) {
                eprintln!("error: {e}");
                std::process::exit(1)
            }
        }
    }
}
