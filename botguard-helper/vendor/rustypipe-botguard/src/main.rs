use std::path::PathBuf;

use clap::Parser;
use const_format::formatcp;
use rustypipe_botguard::{Botguard, Error};

/// Command line API version
const API_VERSION: u16 = 1;

#[derive(Parser)]
#[clap(version = formatcp!("{}\nrustypipe-botguard-api {}", env!("CARGO_PKG_VERSION"), API_VERSION), about)]
struct Cli {
    /// Path to Botguard snapshot file
    #[clap(long)]
    snapshot_file: Option<PathBuf>,
    /// Do not load/store a Botguard snapshot
    #[clap(long)]
    no_snapshot: bool,
    /// User agent for fetching Botguard challenges
    #[clap(long)]
    user_agent: Option<String>,
    /// PO token identifiers (visitor data, video ID)
    idents: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    env_logger::init();
    let cli = Cli::parse();

    let default_filename = "bg_snapshot.bin";
    let snapshot_path = if cli.no_snapshot {
        None
    } else {
        cli.snapshot_file.or_else(|| {
            if let Some(mut data_dir) = dirs::data_dir() {
                data_dir.push("rustypipe");
                if let Err(e) = std::fs::create_dir_all(&data_dir) {
                    log::warn!("could not create data dir: {e}");
                    return None;
                }
                data_dir.push(default_filename);
                Some(data_dir)
            } else {
                Some(PathBuf::from(default_filename))
            }
        })
    };

    let mut bg = Botguard::builder()
        .snapshot_path_opt(snapshot_path.as_deref())
        .user_agent_opt(cli.user_agent.as_deref())
        .init()
        .await?;

    let mut first = true;
    for ident in &cli.idents {
        let po_token = bg.mint_token(ident).await?;
        if first {
            first = false;
        } else {
            print!(" ");
        }
        print!("{po_token}")
    }

    println!(
        " valid_until={} from_snapshot={:?}",
        bg.valid_until().unix_timestamp(),
        bg.is_from_snapshot()
    );

    bg.write_snapshot().await;
    Ok(())
}
