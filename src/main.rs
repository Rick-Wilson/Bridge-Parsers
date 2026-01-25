use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

use bridge_parsers::acbl;
use bridge_parsers::bws;
use bridge_parsers::Direction;
use bridge_parsers::pbn;
use bridge_parsers::xlsx;

#[derive(Parser)]
#[command(name = "bridge-parsers")]
#[command(about = "Read and convert bridge file formats (PBN, BWS)", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert between file formats
    Convert {
        /// Input file (PBN or BWS)
        input: PathBuf,

        /// Output file
        #[arg(short, long)]
        output: PathBuf,

        /// URL to fetch ACBL masterpoint data (e.g., https://d21acbl.org/members/members-d21/)
        #[arg(long)]
        masterpoints_url: Option<String>,
    },

    /// Combine PBN (deals) and BWS (scores) into a single Excel workbook
    Combine {
        /// PBN file containing hand records/deals
        #[arg(long)]
        pbn: PathBuf,

        /// BWS file containing game results and players
        #[arg(long)]
        bws: PathBuf,

        /// Output Excel file
        #[arg(short, long)]
        output: PathBuf,

        /// URL to fetch ACBL masterpoint data (e.g., https://d21acbl.org/members/members-d21/)
        #[arg(long)]
        masterpoints_url: Option<String>,
    },

    /// Display information about a file
    Info {
        /// Input file to inspect
        input: PathBuf,
    },

    /// Validate a file
    Validate {
        /// Input file to validate
        input: PathBuf,
    },
}

fn main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Convert { input, output, masterpoints_url } => {
            convert(&input, &output, masterpoints_url.as_deref())?;
        }
        Commands::Combine { pbn, bws, output, masterpoints_url } => {
            combine(&pbn, &bws, &output, masterpoints_url.as_deref())?;
        }
        Commands::Info { input } => {
            info(&input)?;
        }
        Commands::Validate { input } => {
            validate(&input)?;
        }
    }

    Ok(())
}

fn convert(input: &PathBuf, output: &PathBuf, masterpoints_url: Option<&str>) -> Result<()> {
    let input_ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let output_ext = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // Fetch masterpoint data if URL provided
    let member_data = if let Some(url) = masterpoints_url {
        println!("Fetching masterpoint data from: {}", url);
        match acbl::fetch_member_masterpoints(url) {
            Ok(data) => {
                println!("Loaded {} member records", data.len());
                Some(data)
            }
            Err(e) => {
                println!("Warning: Failed to fetch masterpoint data: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Special case: BWS to Excel preserves game results data
    if input_ext == "bws" && output_ext == "xlsx" {
        println!("Reading BWS file: {}", input.display());
        let data = bws::read_bws(input).context("Failed to read BWS file")?;

        println!("Found {} game results", data.received_data.len());
        println!("Found {} players in this game", data.player_numbers.len());
        if data.has_hand_records() {
            println!("Found {} hand records", data.boards.len());
        }

        println!("Writing Excel file: {}", output.display());
        xlsx::write_bws_to_xlsx_with_masterpoints(&data, output, member_data.as_ref())
            .context("Failed to write Excel file")?;

        println!("Done!");
        return Ok(());
    }

    let boards = match input_ext.as_str() {
        "pbn" => {
            println!("Reading PBN file: {}", input.display());
            pbn::reader::read_pbn_file(input).context("Failed to read PBN file")?
        }
        "bws" => {
            println!("Reading BWS file: {}", input.display());
            let data = bws::read_bws(input).context("Failed to read BWS file")?;

            if data.has_hand_records() {
                println!("Found {} hand records", data.boards.len());
                data.boards
            } else {
                println!("BWS file has no hand records (deals stored in separate PBN file)");
                println!("Found {} game results", data.received_data.len());

                // Create boards from received data (without deals)
                let board_nums = bws::reader::get_board_numbers(&data);
                board_nums
                    .into_iter()
                    .map(|n| {
                        bridge_parsers::Board::new()
                            .with_number(n)
                            .with_dealer(bridge_parsers::dealer_from_board_number(n))
                            .with_vulnerability(bridge_parsers::Vulnerability::from_board_number(n))
                    })
                    .collect()
            }
        }
        _ => {
            anyhow::bail!("Unsupported input format: {}", input_ext);
        }
    };

    println!("Found {} boards", boards.len());

    match output_ext.as_str() {
        "pbn" => {
            println!("Writing PBN file: {}", output.display());
            pbn::writer::write_pbn_file(&boards, output).context("Failed to write PBN file")?;
        }
        "xlsx" => {
            println!("Writing Excel file: {}", output.display());
            xlsx::write_boards_to_xlsx(&boards, output).context("Failed to write Excel file")?;
        }
        _ => {
            anyhow::bail!("Unsupported output format: {}", output_ext);
        }
    }

    println!("Done!");
    Ok(())
}

fn combine(pbn_path: &PathBuf, bws_path: &PathBuf, output: &PathBuf, masterpoints_url: Option<&str>) -> Result<()> {
    // Fetch masterpoint data if URL provided
    let member_data = if let Some(url) = masterpoints_url {
        println!("Fetching masterpoint data from: {}", url);
        match acbl::fetch_member_masterpoints(url) {
            Ok(data) => {
                println!("Loaded {} member records", data.len());
                Some(data)
            }
            Err(e) => {
                println!("Warning: Failed to fetch masterpoint data: {}", e);
                None
            }
        }
    } else {
        None
    };

    // Read PBN file for hand records
    println!("Reading PBN file: {}", pbn_path.display());
    let boards = pbn::reader::read_pbn_file(pbn_path).context("Failed to read PBN file")?;
    println!("Found {} boards with deals", boards.len());

    // Read BWS file for game results
    println!("Reading BWS file: {}", bws_path.display());
    let bws_data = bws::read_bws(bws_path).context("Failed to read BWS file")?;
    println!("Found {} game results", bws_data.received_data.len());
    println!("Found {} players", bws_data.player_numbers.len());

    // Write combined Excel file
    println!("Writing combined Excel file: {}", output.display());
    xlsx::write_combined_to_xlsx(&boards, &bws_data, output, member_data.as_ref())
        .context("Failed to write Excel file")?;

    println!("Done!");
    Ok(())
}

fn info(input: &PathBuf) -> Result<()> {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "pbn" => {
            let boards = pbn::reader::read_pbn_file(input).context("Failed to read PBN file")?;
            println!("PBN File: {}", input.display());
            println!("Boards: {}", boards.len());
            println!();

            for board in &boards {
                print_board_info(board);
            }
        }
        "bws" => {
            let data = bws::read_bws(input).context("Failed to read BWS file")?;
            println!("BWS File: {}", input.display());
            println!();

            println!("Sections: {}", data.sections.len());
            for section in &data.sections {
                println!("  Section {}: {} tables", section.letter.trim(), section.tables);
            }
            println!();

            println!("Players: {}", data.player_names.len());
            for player in data.player_names.iter().take(10) {
                println!("  {} - {}", player.str_id, player.name);
            }
            if data.player_names.len() > 10 {
                println!("  ... and {} more", data.player_names.len() - 10);
            }
            println!();

            println!("Game Results: {}", data.received_data.len());
            let board_nums = bws::reader::get_board_numbers(&data);
            println!("Boards played: {:?}", board_nums);
            println!();

            if data.has_hand_records() {
                println!("Hand Records: {} boards", data.boards.len());
            } else {
                println!("Hand Records: None (deals stored in separate PBN file)");
            }
        }
        _ => {
            anyhow::bail!("Unsupported file format: {}", ext);
        }
    }

    Ok(())
}

fn validate(input: &PathBuf) -> Result<()> {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "pbn" => {
            let boards = pbn::reader::read_pbn_file(input).context("Failed to read PBN file")?;
            println!("PBN file is valid");
            println!("  {} boards", boards.len());

            let mut issues = Vec::new();
            for board in &boards {
                if let Some(num) = board.number {
                    // Check hand sizes
                    for dir in Direction::ALL {
                        let hand = board.deal.hand(dir);
                        let len = hand.len();
                        if len != 13 && len != 0 {
                            issues.push(format!(
                                "Board {}: {} has {} cards (expected 13)",
                                num, dir, len
                            ));
                        }
                    }
                }
            }

            if issues.is_empty() {
                println!("  No issues found");
            } else {
                println!("  Issues found:");
                for issue in issues {
                    println!("    - {}", issue);
                }
            }
        }
        "bws" => {
            let data = bws::read_bws(input).context("Failed to read BWS file")?;
            println!("BWS file is valid");
            println!("  {} sections", data.sections.len());
            println!("  {} players", data.player_names.len());
            println!("  {} results", data.received_data.len());
        }
        _ => {
            anyhow::bail!("Unsupported file format: {}", ext);
        }
    }

    Ok(())
}

fn print_board_info(board: &bridge_parsers::Board) {
    if let Some(num) = board.number {
        println!("Board {}", num);
    }
    if let Some(dealer) = board.dealer {
        println!("  Dealer: {}", dealer);
    }
    println!("  Vulnerable: {}", board.vulnerable);

    let hcp = board.all_hcp();
    println!("  HCP: N={} E={} S={} W={}", hcp[0], hcp[1], hcp[2], hcp[3]);

    // Print compact deal
    for dir in Direction::ALL {
        let hand = board.deal.hand(dir);
        if hand.len() > 0 {
            println!("  {}: {}", dir, hand.to_pbn());
        }
    }
    println!();
}
