//! BBO CSV Tool - Extract cardplay data and perform DD analysis
//!
//! This tool processes CSV files containing TinyURLs to BBO hand records,
//! extracting the complete cardplay sequence and optionally performing
//! double-dummy analysis.

use anyhow::{Context, Result};
use bridge_parsers::lin::parse_lin_from_url;
use bridge_parsers::tinyurl::UrlResolver;
use bridge_parsers::model::{Card, Rank, Suit};
use bridge_solver::{Hands, Solver, PartialTrick, CutoffCache, PatternCache};
use bridge_solver::{NORTH, EAST, SOUTH, WEST, NOTRUMP, SPADE, HEART, DIAMOND, CLUB};
use bridge_solver::cards::card_of;
use clap::{Parser, Subcommand};
use csv::{Reader, ReaderBuilder, Writer, StringRecord};
use rayon::prelude::*;
use regex::Regex;
use std::collections::{HashSet, HashMap};
use std::fs::File;
use std::io::{BufRead, BufReader, Write as IoWrite};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

// ============================================================================
// BBO CSV Preprocessing - Fix malformed quoted fields
// ============================================================================

/// Fix BBO's malformed CSV where the last field (alerts) may contain unescaped quotes.
/// BBO wraps fields containing commas/quotes in double quotes, but doesn't escape
/// internal quotes. Example:
///   ..."2N=Ogust+see+partner"s+response|3S=good+hand,+good+suit"
/// Should become:
///   ..."2N=Ogust+see+partner's+response|3S=good+hand,+good+suit"
fn fix_bbo_csv_line(line: &str) -> String {
    // Quick check: if line doesn't end with a quote, nothing to fix
    if !line.trim_end().ends_with('"') {
        return line.to_string();
    }

    // Find the last field by looking for the pattern: ,"...anything..."
    // We need to find where the last quoted field starts
    lazy_static::lazy_static! {
        // Match: comma, then opening quote, then content, then closing quote at end
        // The content may contain unescaped quotes that we need to fix
        static ref LAST_FIELD_PATTERN: Regex = Regex::new(r#",("[^"]*(?:"[^"]*)*")$"#).unwrap();
    }

    // Alternative simpler approach: find the last comma followed by a quote
    if let Some(last_comma_quote) = line.rfind(",\"") {
        let prefix = &line[..last_comma_quote + 1]; // includes the comma
        let quoted_field = &line[last_comma_quote + 1..]; // starts with quote

        // Check if this quoted field has internal quotes (more than just start/end)
        if quoted_field.len() > 2 && quoted_field.starts_with('"') && quoted_field.trim_end().ends_with('"') {
            let inner = &quoted_field[1..quoted_field.trim_end().len() - 1];

            // If inner content has quotes, replace them with single quotes
            if inner.contains('"') {
                let fixed_inner = inner.replace('"', "'");
                return format!("{}\"{}\"", prefix, fixed_inner);
            }
        }
    }

    line.to_string()
}

/// Read a BBO CSV file and preprocess to fix malformed lines
fn read_bbo_csv_fixed(path: &PathBuf) -> Result<String> {
    let file = File::open(path).context("Failed to open input file")?;
    let reader = BufReader::new(file);
    let mut output = String::new();

    for line in reader.lines() {
        let line = line.context("Failed to read line")?;
        let fixed = fix_bbo_csv_line(&line);
        output.push_str(&fixed);
        output.push('\n');
    }

    Ok(output)
}

#[derive(Parser)]
#[command(name = "bbo-csv")]
#[command(about = "Extract cardplay data from BBO hand records in CSV files")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetch cardplay data from TinyURLs and add to CSV
    FetchCardplay {
        /// Input CSV file
        #[arg(short, long)]
        input: PathBuf,

        /// Output CSV file
        #[arg(short, long)]
        output: PathBuf,

        /// Column name containing the TinyURL/BBO URL
        #[arg(long, default_value = "BBO")]
        url_column: String,

        /// Delay between URL requests in milliseconds
        #[arg(long, default_value = "200")]
        delay_ms: u64,

        /// Number of requests before a longer pause
        #[arg(long, default_value = "10")]
        batch_size: usize,

        /// Duration of the longer pause in milliseconds
        #[arg(long, default_value = "2000")]
        batch_delay_ms: u64,

        /// Resume from previous run (skip rows with existing cardplay data)
        #[arg(long)]
        resume: bool,
    },

    /// Analyze double-dummy cost for each card played
    AnalyzeDd {
        /// Input CSV file (must have Cardplay column and deal columns)
        #[arg(short, long)]
        input: PathBuf,

        /// Output CSV file
        #[arg(short, long)]
        output: PathBuf,

        /// Number of parallel threads (default: number of CPU cores)
        #[arg(short, long)]
        threads: Option<usize>,

        /// Resume from previous run (skip rows with existing DD analysis)
        #[arg(long)]
        resume: bool,

        /// Save progress every N rows
        #[arg(long, default_value = "100")]
        checkpoint_interval: usize,
    },

    /// Anonymize usernames in CSV file.
    ///
    /// Replaces player names with anonymized versions using keyed hashing for
    /// reproducibility. The same name always maps to the same anonymized value
    /// when using the same key. Also processes player names in LIN_URL column.
    Anonymize {
        /// Input CSV file
        #[arg(short, long)]
        input: PathBuf,

        /// Output CSV file
        #[arg(short, long)]
        output: PathBuf,

        /// Secret key for reproducible hashing. Same key produces same mappings.
        /// Set via BBO_ANON_KEY env var to avoid exposing in shell history.
        #[arg(short, long, env = "BBO_ANON_KEY")]
        key: String,

        /// Explicit name mappings to use instead of hashing.
        /// Format: "oldname=NewName,oldname2=NewName2"
        /// Example: --map "JohnDoe=Player1,JaneSmith=Player2"
        #[arg(short, long, default_value = "")]
        map: String,

        /// Columns containing usernames to anonymize.
        /// LIN_URL column is also processed automatically (pn| tag).
        #[arg(long, default_value = "N,S,E,W,Ob name,Dec name,Leader", value_delimiter = ',')]
        columns: Vec<String>,
    },

    /// Analyze DD error statistics by player and role (declaring vs defending)
    Stats {
        /// Input CSV file (must have DD_Analysis column)
        #[arg(short, long)]
        input: PathBuf,

        /// Number of top players to show individually (default: 10)
        #[arg(long, default_value = "10")]
        top_n: usize,

        /// Output detailed CSV with per-player stats
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Display a single hand with DD analysis for spot-checking
    DisplayHand {
        /// Input CSV file (must have Cardplay and DD_Analysis columns)
        #[arg(short, long)]
        input: PathBuf,

        /// Row number to display (1-indexed, not counting header)
        #[arg(short = 'n', long)]
        row: usize,
    },
}

fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::FetchCardplay {
            input,
            output,
            url_column,
            delay_ms,
            batch_size,
            batch_delay_ms,
            resume,
        } => {
            fetch_cardplay(
                &input,
                &output,
                &url_column,
                delay_ms,
                batch_size,
                batch_delay_ms,
                resume,
            )?;
        }
        Commands::AnalyzeDd {
            input,
            output,
            threads,
            resume,
            checkpoint_interval,
        } => {
            analyze_dd(&input, &output, threads, resume, checkpoint_interval)?;
        }
        Commands::Anonymize {
            input,
            output,
            key,
            map,
            columns,
        } => {
            anonymize_csv(&input, &output, &key, &map, &columns)?;
        }
        Commands::Stats {
            input,
            top_n,
            output,
        } => {
            compute_stats(&input, top_n, output.as_ref())?;
        }
        Commands::DisplayHand { input, row } => {
            display_hand(&input, row)?;
        }
    }

    Ok(())
}

fn fetch_cardplay(
    input: &PathBuf,
    output: &PathBuf,
    url_column: &str,
    delay_ms: u64,
    batch_size: usize,
    batch_delay_ms: u64,
    resume: bool,
) -> Result<()> {
    // Read and preprocess input CSV to fix BBO's malformed quoting
    let csv_data = read_bbo_csv_fixed(input)?;
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_data.as_bytes());
    let headers = reader.headers()?.clone();

    // Find the URL column index
    let url_col_idx = headers
        .iter()
        .position(|h| h == url_column)
        .ok_or_else(|| anyhow::anyhow!("Column '{}' not found in CSV", url_column))?;

    // Find the Ref # column for tracking progress
    let ref_col_idx = headers.iter().position(|h| h == "Ref #");

    // Check if input already has Cardplay/LIN_URL columns
    let cardplay_col_idx = headers.iter().position(|h| h == "Cardplay");
    let lin_url_col_idx = headers.iter().position(|h| h == "LIN_URL");

    // If resume mode and output exists, load existing data (ref -> (lin_url, cardplay))
    let existing_data: HashMap<String, (String, String)> = if resume && output.exists() {
        load_existing_cardplay_data(output)?
    } else {
        HashMap::new()
    };

    // Prepare output headers
    let mut output_headers = headers.clone();
    if cardplay_col_idx.is_none() {
        output_headers.push_field("Cardplay");
        output_headers.push_field("LIN_URL");
    }

    // Create URL resolver
    let mut resolver = UrlResolver::with_config(delay_ms, batch_size, batch_delay_ms);

    // Count total rows for progress
    let total_rows = count_csv_rows(input)?;

    // Open output file with flexible field count to handle malformed input rows
    let mut writer = csv::WriterBuilder::new()
        .flexible(true)
        .from_path(output)
        .context("Failed to create output CSV")?;
    writer.write_record(&output_headers)?;

    let mut processed = 0;
    let mut skipped = 0;
    let mut errors = 0;

    for (row_num, result) in reader.records().enumerate() {
        let record = result.context("Failed to read CSV row")?;
        processed += 1;

        // Check if we have existing data for this row (resume mode)
        let ref_id = ref_col_idx.and_then(|i| record.get(i)).unwrap_or("").to_string();
        let existing = existing_data.get(&ref_id);

        // Progress indicator
        eprint!(
            "\r[{}/{}] Processing... ({} errors, {} skipped)    ",
            processed, total_rows, errors, skipped
        );
        std::io::stderr().flush().ok();

        // Use existing data if available and valid, otherwise fetch
        let (cardplay, lin_url) = if let Some((existing_lin, existing_cardplay)) = existing {
            if !existing_cardplay.is_empty() && !existing_cardplay.starts_with("ERROR:") {
                skipped += 1;
                (existing_cardplay.clone(), existing_lin.clone())
            } else {
                // Re-fetch if previous attempt was an error
                fetch_cardplay_for_url(&mut resolver, &record, url_col_idx, row_num, &mut errors)
            }
        } else {
            fetch_cardplay_for_url(&mut resolver, &record, url_col_idx, row_num, &mut errors)
        };

        // Write the row with cardplay/lin_url data
        let mut output_record: Vec<String> = record.iter().map(|s| s.to_string()).collect();

        if let (Some(cp_idx), Some(lu_idx)) = (cardplay_col_idx, lin_url_col_idx) {
            // Update existing columns
            if cp_idx < output_record.len() {
                output_record[cp_idx] = cardplay;
            }
            if lu_idx < output_record.len() {
                output_record[lu_idx] = lin_url;
            }
        } else {
            // Add new columns
            output_record.push(cardplay);
            output_record.push(lin_url);
        }
        writer.write_record(&output_record)?;

        // Flush periodically for crash recovery
        if processed % 100 == 0 {
            writer.flush()?;
        }
    }

    writer.flush()?;
    eprintln!("\nDone! Processed {} rows ({} errors)", processed, errors);

    Ok(())
}

fn process_url(resolver: &mut UrlResolver, url: &str) -> Result<(String, String)> {
    // Resolve the URL if it's a shortener
    let resolved_url = if url.contains("tinyurl.com") || url.contains("bit.ly") {
        resolver.resolve(url)?
    } else {
        url.to_string()
    };

    // Parse the LIN data
    let lin_data = parse_lin_from_url(&resolved_url)?;

    // Format cardplay
    let cardplay = lin_data.format_cardplay_by_trick();

    Ok((cardplay, resolved_url))
}

/// Helper to fetch cardplay for a URL, handling errors
fn fetch_cardplay_for_url(
    resolver: &mut UrlResolver,
    record: &StringRecord,
    url_col_idx: usize,
    row_num: usize,
    errors: &mut usize,
) -> (String, String) {
    let url = record.get(url_col_idx).unwrap_or("").trim();

    if url.is_empty() {
        return (String::new(), String::new());
    }

    match process_url(resolver, url) {
        Ok((cp, lu)) => (cp, lu),
        Err(e) => {
            log::warn!("Row {}: Error processing URL '{}': {}", row_num + 1, url, e);
            *errors += 1;

            // Check if rate limited and need to pause
            if e.to_string().contains("Rate limited") {
                eprintln!("\nRate limited - pausing for 60 seconds...");
                std::thread::sleep(std::time::Duration::from_secs(60));
                resolver.reset_batch();
            }

            (format!("ERROR: {}", e), String::new())
        }
    }
}

/// Load existing cardplay data from output file for resume
fn load_existing_cardplay_data(output: &PathBuf) -> Result<HashMap<String, (String, String)>> {
    let mut data = HashMap::new();
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(output)?;

    let headers = reader.headers()?.clone();
    let ref_idx = headers.iter().position(|h| h == "Ref #");
    let lin_url_idx = headers.iter().position(|h| h == "LIN_URL");
    let cardplay_idx = headers.iter().position(|h| h == "Cardplay");

    if ref_idx.is_none() || cardplay_idx.is_none() {
        return Ok(data);
    }

    let ref_idx = ref_idx.unwrap();
    let cardplay_idx = cardplay_idx.unwrap();

    for result in reader.records() {
        let record = result?;
        let ref_id = record.get(ref_idx).unwrap_or("").to_string();
        let lin_url = lin_url_idx
            .and_then(|i| record.get(i))
            .unwrap_or("")
            .to_string();
        let cardplay = record.get(cardplay_idx).unwrap_or("").to_string();

        if !ref_id.is_empty() {
            data.insert(ref_id, (lin_url, cardplay));
        }
    }

    Ok(data)
}

fn load_existing_refs(output: &PathBuf, column: &str) -> Result<HashSet<String>> {
    let mut refs = HashSet::new();
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(output)?;

    let headers = reader.headers()?.clone();
    let ref_idx = headers.iter().position(|h| h == "Ref #");
    let col_idx = headers.iter().position(|h| h == column);

    if ref_idx.is_none() || col_idx.is_none() {
        return Ok(refs);
    }

    let ref_idx = ref_idx.unwrap();
    let col_idx = col_idx.unwrap();

    for result in reader.records() {
        let record = result?;
        let ref_id = record.get(ref_idx).unwrap_or("");
        let value = record.get(col_idx).unwrap_or("");

        // Only consider it "done" if value is non-empty and not an error
        if !value.is_empty() && !value.starts_with("ERROR:") {
            refs.insert(ref_id.to_string());
        }
    }

    Ok(refs)
}

fn count_csv_rows(path: &PathBuf) -> Result<usize> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    // Subtract 1 for header row
    Ok(reader.lines().count().saturating_sub(1))
}

// ============================================================================
// DD Analysis Implementation
// ============================================================================

/// Represents a row to be processed for DD analysis
#[derive(Clone)]
struct DdWorkItem {
    row_idx: usize,
    #[allow(dead_code)]
    ref_id: String,
    deal_pbn: String,
    cardplay: String,
    contract: String,
    declarer: String,
}

fn analyze_dd(
    input: &PathBuf,
    output: &PathBuf,
    threads: Option<usize>,
    resume: bool,
    checkpoint_interval: usize,
) -> Result<()> {
    // Configure thread pool
    if let Some(n) = threads {
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()
            .ok(); // Ignore error if already initialized
    }

    // Read input CSV with flexible field count to handle malformed rows
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(input)
        .context("Failed to open input CSV")?;
    let headers = reader.headers()?.clone();

    // Find required columns
    let col_indices = find_required_columns(&headers)?;

    // Check if DD_Analysis column already exists
    let dd_col_exists = headers.iter().any(|h| h == "DD_Analysis");

    // Load existing results if resuming
    let existing_refs: HashSet<String> = if resume && output.exists() {
        load_existing_refs(output, "DD_Analysis")?
    } else {
        HashSet::new()
    };

    // Prepare output headers
    let mut output_headers = headers.clone();
    if !dd_col_exists {
        output_headers.push_field("DD_Analysis");
    }

    // Collect all rows and prepare work items
    let mut all_records: Vec<StringRecord> = Vec::new();
    let mut work_items: Vec<DdWorkItem> = Vec::new();

    for (row_idx, result) in reader.records().enumerate() {
        let record = result.context("Failed to read CSV row")?;
        all_records.push(record.clone());

        let ref_id = record.get(col_indices.ref_col).unwrap_or("").to_string();

        // Skip if already processed (resume mode)
        if resume && existing_refs.contains(&ref_id) {
            continue;
        }

        // Get the cardplay
        let cardplay = record.get(col_indices.cardplay_col).unwrap_or("").to_string();

        if cardplay.is_empty() || cardplay.starts_with("ERROR:") {
            continue;
        }

        // Extract deal, contract, and declarer from row
        if let Some(row_data) = extract_row_data(&record, &col_indices) {
            work_items.push(DdWorkItem {
                row_idx,
                ref_id,
                deal_pbn: row_data.deal_pbn,
                cardplay,
                contract: row_data.contract,
                declarer: row_data.declarer,
            });
        }
    }

    let total_rows = all_records.len();
    let to_process = work_items.len();

    eprintln!(
        "Found {} rows, {} need DD analysis ({} already done)",
        total_rows,
        to_process,
        total_rows - to_process
    );

    if to_process == 0 {
        eprintln!("Nothing to do!");
        return Ok(());
    }

    // Progress tracking
    let processed_count = AtomicUsize::new(0);
    let error_count = AtomicUsize::new(0);

    // Store results in a thread-safe map
    let results: Mutex<HashMap<usize, String>> = Mutex::new(HashMap::new());

    // Process work items in parallel
    work_items.par_iter().for_each(|item| {
        let dd_analysis = match compute_dd_analysis(item) {
            Ok(analysis) => analysis,
            Err(e) => {
                error_count.fetch_add(1, Ordering::Relaxed);
                log::warn!("Row {}: DD analysis error: {}", item.row_idx + 1, e);
                format!("ERROR: {}", e)
            }
        };

        // Store result
        results.lock().unwrap().insert(item.row_idx, dd_analysis);

        // Update progress
        let count = processed_count.fetch_add(1, Ordering::Relaxed) + 1;
        if count % 10 == 0 || count == to_process {
            let errors = error_count.load(Ordering::Relaxed);
            eprint!(
                "\r[{}/{}] Analyzing DD... ({} errors)    ",
                count, to_process, errors
            );
            std::io::stderr().flush().ok();
        }
    });

    eprintln!(); // New line after progress

    // Write output
    let results_map = results.into_inner().unwrap();
    let mut writer = Writer::from_path(output).context("Failed to create output CSV")?;
    writer.write_record(&output_headers)?;

    for (row_idx, record) in all_records.iter().enumerate() {
        let mut output_record = record.clone();

        if !dd_col_exists {
            let dd_analysis = results_map.get(&row_idx).cloned().unwrap_or_default();
            output_record.push_field(&dd_analysis);
        }

        writer.write_record(&output_record)?;

        // Checkpoint
        if (row_idx + 1) % checkpoint_interval == 0 {
            writer.flush()?;
        }
    }

    writer.flush()?;

    let errors = error_count.load(Ordering::Relaxed);
    eprintln!(
        "Done! Analyzed {} rows ({} errors)",
        to_process, errors
    );

    Ok(())
}

/// Column indices for required fields
struct ColumnIndices {
    ref_col: usize,
    cardplay_col: usize,
    contract_col: Option<usize>,
    declarer_col: Option<usize>,
    lin_url_col: Option<usize>,
    // Hand columns (actual PBN-style hand data, not player names)
    north_col: Option<usize>,
    south_col: Option<usize>,
    east_col: Option<usize>,
    west_col: Option<usize>,
}

fn find_required_columns(headers: &StringRecord) -> Result<ColumnIndices> {
    let find = |name: &str| -> Result<usize> {
        headers
            .iter()
            .position(|h| h == name)
            .ok_or_else(|| anyhow::anyhow!("Required column '{}' not found", name))
    };

    let find_optional = |name: &str| -> Option<usize> {
        headers.iter().position(|h| h == name)
    };

    let lin_url_col = find_optional("LIN_URL");
    let contract_col = find_optional("Con");
    let declarer_col = find_optional("Dec");

    // We need either LIN_URL (which has everything) or Con+Dec columns
    if lin_url_col.is_none() && (contract_col.is_none() || declarer_col.is_none()) {
        return Err(anyhow::anyhow!(
            "CSV must have either 'LIN_URL' column or both 'Con' and 'Dec' columns"
        ));
    }

    Ok(ColumnIndices {
        ref_col: find("Ref #")?,
        cardplay_col: find("Cardplay")?,
        contract_col,
        declarer_col,
        lin_url_col,
        // Look for hand columns (might be PBN-style hands or player names)
        north_col: find_optional("North").or_else(|| find_optional("N_Hand")),
        south_col: find_optional("South").or_else(|| find_optional("S_Hand")),
        east_col: find_optional("East").or_else(|| find_optional("E_Hand")),
        west_col: find_optional("West").or_else(|| find_optional("W_Hand")),
    })
}

/// Data extracted from a row for DD analysis
struct RowData {
    deal_pbn: String,
    contract: String,
    declarer: String,
}

/// Extract deal, contract, and declarer from a CSV row
/// Prefers explicit columns (Con, Dec, hand columns) but falls back to LIN_URL
fn extract_row_data(record: &StringRecord, cols: &ColumnIndices) -> Option<RowData> {
    // Try to get contract and declarer from explicit columns first
    let contract_from_col = cols.contract_col.and_then(|i| record.get(i)).map(|s| s.to_string());
    let declarer_from_col = cols.declarer_col.and_then(|i| record.get(i)).map(|s| s.to_string());

    // Try to get deal from hand columns (if they contain actual hand data)
    let deal_from_hands = build_deal_from_hand_cols(record, cols);

    // If we have hand columns with valid data, use them
    if let Some(deal_pbn) = deal_from_hands {
        if let (Some(contract), Some(declarer)) = (contract_from_col.clone(), declarer_from_col.clone()) {
            if !contract.is_empty() && !declarer.is_empty() {
                return Some(RowData {
                    deal_pbn,
                    contract,
                    declarer,
                });
            }
        }
    }

    // Fall back to LIN_URL
    if let Some(lin_url_col) = cols.lin_url_col {
        if let Some(url) = record.get(lin_url_col) {
            if !url.is_empty() {
                if let Ok(lin_data) = parse_lin_from_url(url) {
                    let deal_pbn = lin_data.deal.to_pbn(bridge_parsers::model::Direction::North);

                    // Use explicit columns if available, otherwise extract from LIN
                    let contract = contract_from_col
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| extract_contract_from_lin(&lin_data));
                    let declarer = declarer_from_col
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| extract_declarer_from_lin(&lin_data));

                    if !contract.is_empty() && !declarer.is_empty() {
                        return Some(RowData {
                            deal_pbn,
                            contract,
                            declarer,
                        });
                    }
                }
            }
        }
    }

    None
}

/// Try to build a PBN deal from hand columns
/// Returns None if columns don't exist or don't contain valid hand data
fn build_deal_from_hand_cols(record: &StringRecord, cols: &ColumnIndices) -> Option<String> {
    let get_hand = |col: Option<usize>| -> Option<&str> {
        col.and_then(|i| record.get(i)).filter(|s| !s.is_empty())
    };

    let north = get_hand(cols.north_col)?;
    let south = get_hand(cols.south_col)?;
    let east = get_hand(cols.east_col)?;
    let west = get_hand(cols.west_col)?;

    // Check if these look like PBN hands (contain dots for suit separators)
    // Player names won't have dots
    if !north.contains('.') || !south.contains('.') {
        return None;
    }

    Some(format!("N:{} {} {} {}", north, east, south, west))
}

/// Extract contract from LIN auction data
fn extract_contract_from_lin(lin_data: &bridge_parsers::lin::LinData) -> String {
    // Walk through auction to find final contract
    let mut level = 0u8;
    let mut suit = String::new();
    let mut doubled = false;
    let mut redoubled = false;

    for bid in &lin_data.auction {
        let bid_str = bid.bid.to_uppercase();

        if bid_str == "P" || bid_str == "PASS" {
            continue;
        } else if bid_str == "D" || bid_str == "X" || bid_str == "DBL" {
            doubled = true;
            redoubled = false;
        } else if bid_str == "R" || bid_str == "XX" || bid_str == "RDBL" {
            redoubled = true;
        } else if let Some(c) = bid_str.chars().next() {
            if c.is_ascii_digit() {
                level = c.to_digit(10).unwrap_or(0) as u8;
                suit = bid_str[1..].to_string();
                doubled = false;
                redoubled = false;
            }
        }
    }

    if level == 0 {
        return String::new(); // Passed out
    }

    let mut contract = format!("{}{}", level, suit);
    if redoubled {
        contract.push_str("XX");
    } else if doubled {
        contract.push_str("X");
    }

    contract
}

/// Extract declarer from LIN data by finding who holds the opening lead card
/// This is more reliable than parsing the auction (which has artificial bids)
fn extract_declarer_from_lin(lin_data: &bridge_parsers::lin::LinData) -> String {
    use bridge_parsers::model::Direction;

    // If there's cardplay, use the opening lead to determine the leader
    // Then declarer is to the right of the leader
    if !lin_data.play.is_empty() {
        let opening_lead = &lin_data.play[0];

        // Find which hand has this card
        for dir in Direction::all() {
            let hand = lin_data.deal.hand(dir);
            if hand.holding(opening_lead.suit).contains(opening_lead.rank) {
                // This player led, so declarer is to their right
                let declarer = match dir {
                    Direction::North => "W", // N leads means W declares
                    Direction::East => "N",  // E leads means N declares
                    Direction::South => "E", // S leads means E declares
                    Direction::West => "S",  // W leads means S declares
                };
                return declarer.to_string();
            }
        }
    }

    // Fallback: try to determine from auction
    extract_declarer_from_auction(lin_data)
}

/// Fallback: Extract declarer from auction (may be wrong for artificial bids)
fn extract_declarer_from_auction(lin_data: &bridge_parsers::lin::LinData) -> String {
    let mut level = 0u8;
    let mut final_suit = String::new();
    let mut final_bidder_idx = 0usize;

    let dealer = lin_data.dealer;

    for (i, bid) in lin_data.auction.iter().enumerate() {
        let bid_str = bid.bid.to_uppercase();

        if bid_str == "P" || bid_str == "PASS" || bid_str == "D" || bid_str == "X"
            || bid_str == "R" || bid_str == "XX" || bid_str == "DBL" || bid_str == "RDBL" {
            continue;
        }

        if let Some(c) = bid_str.chars().next() {
            if c.is_ascii_digit() {
                level = c.to_digit(10).unwrap_or(0) as u8;
                final_suit = bid_str[1..].to_string();
                final_bidder_idx = i;
            }
        }
    }

    if level == 0 {
        return String::new(); // Passed out
    }

    // The declarer is the first person on the declaring partnership to bid the suit
    let declaring_side = (dealer as usize + final_bidder_idx) % 4;
    let declaring_partnership = declaring_side % 2; // 0 = N/S, 1 = E/W

    // Find first bid of final suit by the declaring partnership
    for (i, bid) in lin_data.auction.iter().enumerate() {
        let bid_str = bid.bid.to_uppercase();
        let bidder = (dealer as usize + i) % 4;

        if bidder % 2 != declaring_partnership {
            continue;
        }

        if let Some(c) = bid_str.chars().next() {
            if c.is_ascii_digit() {
                let bid_suit = &bid_str[1..];
                if bid_suit == final_suit {
                    return match bidder {
                        0 => "N".to_string(),
                        1 => "E".to_string(),
                        2 => "S".to_string(),
                        3 => "W".to_string(),
                        _ => String::new(),
                    };
                }
            }
        }
    }

    // Last fallback: just return the final bidder
    match (dealer as usize + final_bidder_idx) % 4 {
        0 => "N".to_string(),
        1 => "E".to_string(),
        2 => "S".to_string(),
        3 => "W".to_string(),
        _ => String::new(),
    }
}

/// Compute DD analysis for a single work item
///
/// For each card played, computes the DD cost of the actual play vs optimal.
/// DD cost represents tricks lost by suboptimal play (0 = optimal or equivalent).
/// Output format: T1:c1,c2,c3,c4|T2:c1,c2,c3,c4|... where each c is the cost for that card
fn compute_dd_analysis(item: &DdWorkItem) -> Result<String> {
    // Parse the deal
    let hands = Hands::from_pbn(&item.deal_pbn)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse deal: {}", item.deal_pbn))?;

    // Parse the contract to get trump suit
    let trump = parse_trump(&item.contract)?;

    // Parse declarer to get leader
    let declarer_seat = parse_declarer(&item.declarer)?;
    let initial_leader = (declarer_seat + 1) % 4; // Left of declarer leads

    // Determine if declarer is N-S or E-W
    // N=1, S=3 -> odd seats = N-S partnership
    // W=0, E=2 -> even seats = E-W partnership
    let declarer_is_ns = declarer_seat == NORTH || declarer_seat == SOUTH;

    // Parse cardplay into tricks
    let tricks = parse_cardplay(&item.cardplay)?;

    if tricks.is_empty() {
        return Ok(String::new());
    }

    // Analyze card-by-card
    let mut trick_results: Vec<String> = Vec::new();
    let mut current_hands = hands;
    let mut current_leader = initial_leader;

    // Caches for solver (reused across all solves for this hand)
    let mut cutoff_cache = CutoffCache::new(16);
    let mut pattern_cache = PatternCache::new(16);

    for (trick_num, trick) in tricks.iter().enumerate() {
        let mut card_costs: Vec<u8> = Vec::new();
        let mut cards_in_trick: Vec<(usize, usize)> = Vec::new(); // (seat, card)
        let mut partial_trick = PartialTrick::new();
        let mut seat = current_leader;

        for (card_idx, card) in trick.iter().enumerate() {
            let cards_remaining = current_hands.num_tricks() as u8;

            // DD before this card is played (accounting for partial trick state)
            let ns_before = if card_idx == 0 {
                // Start of trick - normal solve
                solve_position_with_caches(&current_hands, trump, current_leader, &mut cutoff_cache, &mut pattern_cache)
            } else {
                // Mid-trick - use partial trick solver
                solve_mid_trick_position(&current_hands, trump, current_leader, &partial_trick, &mut cutoff_cache, &mut pattern_cache)
            };
            let declarer_before = if declarer_is_ns {
                ns_before
            } else {
                cards_remaining.saturating_sub(ns_before)
            };

            // Convert and remove the card
            let solver_card = bridge_card_to_solver(*card)?;
            cards_in_trick.push((seat, solver_card));
            current_hands[seat].remove(solver_card);

            // Add to partial trick for mid-trick solving
            partial_trick.add(solver_card, seat);

            // DD after this card is played
            let declarer_after = if card_idx == 3 && cards_in_trick.len() == 4 {
                // Trick complete - find winner
                let winner = determine_trick_winner(&cards_in_trick, trump, current_leader);
                let declarer_won = if declarer_is_ns {
                    winner == NORTH || winner == SOUTH
                } else {
                    winner == EAST || winner == WEST
                };

                if current_hands.num_tricks() == 0 {
                    // Last trick
                    if declarer_won { 1 } else { 0 }
                } else {
                    let ns_after = solve_position_with_caches(&current_hands, trump, winner, &mut cutoff_cache, &mut pattern_cache);
                    let remaining = current_hands.num_tricks() as u8;
                    if declarer_is_ns {
                        ns_after + if declarer_won { 1 } else { 0 }
                    } else {
                        remaining.saturating_sub(ns_after) + if declarer_won { 1 } else { 0 }
                    }
                }
            } else {
                // Partial trick - solve with partial trick state
                let ns_after = solve_mid_trick_position(&current_hands, trump, current_leader, &partial_trick, &mut cutoff_cache, &mut pattern_cache);
                let remaining = current_hands.num_tricks() as u8;
                if declarer_is_ns {
                    ns_after
                } else {
                    remaining.saturating_sub(ns_after)
                }
            };

            // Cost depends on who is playing:
            // - Declarer/dummy: cost if declarer's position got worse (declarer_before > declarer_after)
            // - Defender: cost if declarer's position got better (declarer_after > declarer_before)
            let is_declarer_side = if declarer_is_ns {
                seat == NORTH || seat == SOUTH
            } else {
                seat == EAST || seat == WEST
            };

            let cost = if is_declarer_side {
                // Declarer error: lost tricks
                declarer_before.saturating_sub(declarer_after)
            } else {
                // Defender error: declarer gained tricks
                declarer_after.saturating_sub(declarer_before)
            };
            card_costs.push(cost);

            seat = (seat + 1) % 4;
        }

        // Format: T1:c1,c2,c3,c4
        let costs_str = card_costs.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(",");
        trick_results.push(format!("T{}:{}", trick_num + 1, costs_str));

        // Update leader for next trick
        if cards_in_trick.len() == 4 {
            let winner = determine_trick_winner(&cards_in_trick, trump, current_leader);
            current_leader = winner;
        }
    }

    Ok(trick_results.join("|"))
}

/// Solve a position and return NS tricks (with caches)
fn solve_position_with_caches(
    hands: &Hands,
    trump: usize,
    leader: usize,
    cutoff_cache: &mut CutoffCache,
    pattern_cache: &mut PatternCache,
) -> u8 {
    if hands.num_tricks() == 0 {
        return 0;
    }

    let solver = Solver::new(*hands, trump, leader);
    solver.solve_with_caches(cutoff_cache, pattern_cache)
}

/// Solve a mid-trick position and return NS tricks
fn solve_mid_trick_position(
    hands: &Hands,
    trump: usize,
    _leader: usize,  // Unused - leader is derived from partial_trick
    partial_trick: &PartialTrick,
    cutoff_cache: &mut CutoffCache,
    pattern_cache: &mut PatternCache,
) -> u8 {
    // Use new_mid_trick to correctly handle mid-trick positions
    // It computes num_tricks from max hand size (not fixed seat) and
    // derives leader from the partial_trick
    if let Some(solver) = Solver::new_mid_trick(*hands, trump, partial_trick) {
        solver.solve_mid_trick(cutoff_cache, pattern_cache, partial_trick)
    } else if let Some(leader) = partial_trick.leader() {
        // Fallback: use regular solve if new_mid_trick fails
        let solver = Solver::new(*hands, trump, leader);
        solver.solve_with_caches(cutoff_cache, pattern_cache)
    } else {
        0
    }
}

/// Solve a position and return NS tricks (simple version without caches)
fn solve_position(hands: &Hands, trump: usize, leader: usize) -> u8 {
    if hands.num_tricks() == 0 {
        return 0;
    }

    let solver = Solver::new(*hands, trump, leader);
    solver.solve()
}

/// Parse trump suit from contract string (e.g., "4S", "3NT", "6H")
fn parse_trump(contract: &str) -> Result<usize> {
    let contract = contract.trim().to_uppercase();

    if contract.contains("NT") || contract.contains("N") && !contract.contains("S") {
        return Ok(NOTRUMP);
    }

    // Find suit letter
    for c in contract.chars() {
        match c {
            'S' => return Ok(SPADE),
            'H' => return Ok(HEART),
            'D' => return Ok(DIAMOND),
            'C' => return Ok(CLUB),
            _ => continue,
        }
    }

    Err(anyhow::anyhow!("Could not parse trump from contract: {}", contract))
}

/// Parse declarer from direction string
fn parse_declarer(declarer: &str) -> Result<usize> {
    match declarer.trim().to_uppercase().chars().next() {
        Some('N') => Ok(NORTH),
        Some('E') => Ok(EAST),
        Some('S') => Ok(SOUTH),
        Some('W') => Ok(WEST),
        _ => Err(anyhow::anyhow!("Invalid declarer: {}", declarer)),
    }
}

/// Parse cardplay string into tricks
/// Format: "D2-DA-D6-D5|S3-S2-SQ-SA|..."
fn parse_cardplay(cardplay: &str) -> Result<Vec<Vec<Card>>> {
    let mut tricks = Vec::new();

    for trick_str in cardplay.split('|') {
        if trick_str.is_empty() {
            continue;
        }

        let mut trick = Vec::new();
        for card_str in trick_str.split(' ') {
            let card = parse_card_str(card_str)?;
            trick.push(card);
        }

        if !trick.is_empty() {
            tricks.push(trick);
        }
    }

    Ok(tricks)
}

/// Parse a card string like "SA", "D2", "HK"
fn parse_card_str(s: &str) -> Result<Card> {
    let s = s.trim();
    if s.len() < 2 {
        return Err(anyhow::anyhow!("Invalid card: {}", s));
    }

    let mut chars = s.chars();
    let suit_char = chars.next().unwrap();
    let rank_char = chars.next().unwrap();

    let suit = match suit_char.to_ascii_uppercase() {
        'S' => Suit::Spades,
        'H' => Suit::Hearts,
        'D' => Suit::Diamonds,
        'C' => Suit::Clubs,
        _ => return Err(anyhow::anyhow!("Invalid suit: {}", suit_char)),
    };

    let rank = Rank::from_pbn_char(rank_char)
        .ok_or_else(|| anyhow::anyhow!("Invalid rank: {}", rank_char))?;

    Ok(Card::new(suit, rank))
}

/// Convert bridge-parsers Card to bridge-solver card index
fn bridge_card_to_solver(card: Card) -> Result<usize> {
    let suit = match card.suit {
        Suit::Spades => SPADE,
        Suit::Hearts => HEART,
        Suit::Diamonds => DIAMOND,
        Suit::Clubs => CLUB,
    };

    let rank = match card.rank {
        Rank::Ace => 12,
        Rank::King => 11,
        Rank::Queen => 10,
        Rank::Jack => 9,
        Rank::Ten => 8,
        Rank::Nine => 7,
        Rank::Eight => 6,
        Rank::Seven => 5,
        Rank::Six => 4,
        Rank::Five => 3,
        Rank::Four => 2,
        Rank::Three => 1,
        Rank::Two => 0,
    };

    Ok(card_of(suit, rank))
}

/// Determine winner of a trick
fn determine_trick_winner(
    cards: &[(usize, usize)], // (seat, card)
    trump: usize,
    leader: usize,
) -> usize {
    use bridge_solver::cards::suit_of;

    let _lead_suit = suit_of(cards[0].1);
    let mut winner_idx = 0;
    let mut winning_card = cards[0].1;

    for (i, (_seat, card)) in cards.iter().enumerate().skip(1) {
        let card_suit = suit_of(*card);

        // Check if this card beats the current winner
        let beats = if card_suit == suit_of(winning_card) {
            // Same suit - higher card wins (lower index = higher rank)
            *card < winning_card
        } else if card_suit == trump && trump < NOTRUMP {
            // Trump beats non-trump
            suit_of(winning_card) != trump
        } else {
            false
        };

        if beats {
            winner_idx = i;
            winning_card = *card;
        }
    }

    // Return the actual seat
    (leader + winner_idx) % 4
}

// ============================================================================
// Anonymize Implementation
// ============================================================================

/// Common first names (mix of male and female)
const FIRST_NAMES: &[&str] = &[
    "Aaron", "Abigail", "Adam", "Adrian", "Aiden", "Alex", "Alice", "Allison",
    "Amanda", "Amber", "Amy", "Andrea", "Andrew", "Angela", "Anna", "Anthony",
    "Ashley", "Austin", "Barbara", "Benjamin", "Beth", "Brandon", "Brenda",
    "Brian", "Brittany", "Bruce", "Bryan", "Caleb", "Cameron", "Carl", "Carlos",
    "Carol", "Caroline", "Catherine", "Charles", "Charlotte", "Chelsea", "Chris",
    "Christina", "Christine", "Christopher", "Cindy", "Claire", "Clara", "Cody",
    "Colin", "Connor", "Craig", "Crystal", "Cynthia", "Dale", "Daniel", "Danielle",
    "Darren", "David", "Dawn", "Deborah", "Denise", "Dennis", "Derek", "Diana",
    "Diane", "Donald", "Donna", "Dorothy", "Douglas", "Dylan", "Edward", "Eileen",
    "Eleanor", "Elizabeth", "Ellen", "Emily", "Emma", "Eric", "Erica", "Erin",
    "Ethan", "Eugene", "Eva", "Evan", "Evelyn", "Frances", "Francis", "Frank",
    "Gabriel", "Gary", "George", "Gerald", "Gloria", "Grace", "Gregory", "Hannah",
    "Harold", "Harry", "Heather", "Helen", "Henry", "Holly", "Howard", "Ian",
    "Isaac", "Isabella", "Jack", "Jacob", "Jacqueline", "Jake", "James", "Jamie",
    "Jane", "Janet", "Janice", "Jason", "Jean", "Jeffrey", "Jennifer", "Jeremy",
    "Jerry", "Jesse", "Jessica", "Jill", "Joan", "Joe", "Joel", "John", "Jonathan",
    "Jordan", "Jose", "Joseph", "Joshua", "Joyce", "Juan", "Judith", "Julia",
    "Julie", "Justin", "Karen", "Katherine", "Kathleen", "Kathryn", "Katie",
    "Keith", "Kelly", "Kenneth", "Kevin", "Kim", "Kimberly", "Kyle", "Larry",
    "Laura", "Lauren", "Lawrence", "Leah", "Leonard", "Leslie", "Lillian", "Linda",
    "Lindsay", "Lisa", "Logan", "Lori", "Louis", "Lucas", "Lucy", "Luke", "Lynn",
    "Madison", "Margaret", "Maria", "Marie", "Marilyn", "Mark", "Martha", "Martin",
    "Mary", "Mason", "Matthew", "Megan", "Melanie", "Melissa", "Michael", "Michelle",
    "Mike", "Mildred", "Monica", "Nancy", "Natalie", "Nathan", "Nicholas", "Nicole",
    "Noah", "Norma", "Oliver", "Olivia", "Oscar", "Pamela", "Patricia", "Patrick",
    "Paul", "Paula", "Peggy", "Peter", "Philip", "Rachel", "Ralph", "Randy",
    "Raymond", "Rebecca", "Regina", "Richard", "Robert", "Robin", "Roger", "Ronald",
    "Rose", "Roy", "Russell", "Ruth", "Ryan", "Samantha", "Samuel", "Sandra",
    "Sara", "Sarah", "Scott", "Sean", "Sharon", "Shawn", "Sheila", "Shirley",
    "Sophia", "Stephanie", "Stephen", "Steve", "Steven", "Susan", "Tammy", "Teresa",
    "Terry", "Theresa", "Thomas", "Tiffany", "Timothy", "Tina", "Todd", "Tom",
    "Tony", "Tracy", "Travis", "Tyler", "Valerie", "Vanessa", "Victor", "Victoria",
    "Vincent", "Virginia", "Walter", "Wanda", "Wayne", "Wendy", "William", "Willie",
    "Zachary",
];

/// Common surnames
const SURNAMES: &[&str] = &[
    "Adams", "Allen", "Anderson", "Bailey", "Baker", "Barnes", "Bell", "Bennett",
    "Brooks", "Brown", "Bryant", "Butler", "Campbell", "Carter", "Clark", "Coleman",
    "Collins", "Cook", "Cooper", "Cox", "Cruz", "Davis", "Diaz", "Edwards", "Evans",
    "Fisher", "Flores", "Ford", "Foster", "Garcia", "Gibson", "Gomez", "Gonzalez",
    "Gordon", "Graham", "Gray", "Green", "Griffin", "Hall", "Hamilton", "Harris",
    "Harrison", "Hayes", "Henderson", "Hernandez", "Hill", "Holmes", "Howard",
    "Hughes", "Hunt", "Jackson", "James", "Jenkins", "Johnson", "Jones", "Jordan",
    "Kelly", "Kennedy", "Kim", "King", "Lee", "Lewis", "Long", "Lopez", "Marshall",
    "Martin", "Martinez", "Mason", "Matthews", "Mcdonald", "Miller", "Mitchell",
    "Moore", "Morales", "Morgan", "Morris", "Murphy", "Murray", "Nelson", "Nguyen",
    "Ortiz", "Owens", "Parker", "Patterson", "Perez", "Perry", "Peterson", "Phillips",
    "Powell", "Price", "Ramirez", "Reed", "Reyes", "Reynolds", "Richardson", "Rivera",
    "Roberts", "Robinson", "Rodriguez", "Rogers", "Ross", "Russell", "Sanchez",
    "Sanders", "Scott", "Simmons", "Smith", "Stewart", "Sullivan", "Taylor", "Thomas",
    "Thompson", "Torres", "Turner", "Walker", "Wallace", "Ward", "Washington",
    "Watson", "West", "White", "Williams", "Wilson", "Wood", "Wright", "Young",
];

/// Anonymizer that maps usernames to fake names using keyed hashing
struct Anonymizer {
    key: String,
    explicit_maps: HashMap<String, String>,
    generated_maps: HashMap<String, String>,
    used_names: HashSet<String>,
    name_count: usize,
}

impl Anonymizer {
    fn new(key: &str, explicit_map_str: &str) -> Self {
        let mut explicit_maps = HashMap::new();
        let mut used_names = HashSet::new();

        // Parse explicit mappings
        for mapping in explicit_map_str.split(',') {
            let mapping = mapping.trim();
            if mapping.is_empty() {
                continue;
            }
            if let Some((old, new)) = mapping.split_once('=') {
                let old = old.trim().to_lowercase();
                let new = new.trim().to_string();
                used_names.insert(new.clone());
                explicit_maps.insert(old, new);
            }
        }

        Anonymizer {
            key: key.to_string(),
            explicit_maps,
            generated_maps: HashMap::new(),
            used_names,
            name_count: 0,
        }
    }

    /// Get anonymous name for a username
    fn anonymize(&mut self, username: &str) -> String {
        let username_lower = username.to_lowercase();

        // Check explicit mapping first
        if let Some(mapped) = self.explicit_maps.get(&username_lower) {
            return mapped.clone();
        }

        // Check if we already generated a name for this user
        if let Some(mapped) = self.generated_maps.get(&username_lower) {
            return mapped.clone();
        }

        // Generate a new name using keyed hash
        let new_name = self.generate_name(&username_lower);
        self.generated_maps.insert(username_lower, new_name.clone());
        new_name
    }

    /// Generate a unique name using keyed hash
    fn generate_name(&mut self, username: &str) -> String {
        // Simple keyed hash: combine key + username, then hash
        let combined = format!("{}:{}", self.key, username);
        let hash = self.simple_hash(&combined);

        // Use hash to pick first name and surname
        let first_idx = (hash % FIRST_NAMES.len() as u64) as usize;
        let surname_idx = ((hash / FIRST_NAMES.len() as u64) % SURNAMES.len() as u64) as usize;

        let mut candidate = format!("{}_{}", FIRST_NAMES[first_idx], SURNAMES[surname_idx]);

        // If name is already used (collision or explicit), add a number
        let mut suffix = 2;
        while self.used_names.contains(&candidate) {
            candidate = format!("{}_{}_{}", FIRST_NAMES[first_idx], SURNAMES[surname_idx], suffix);
            suffix += 1;
        }

        self.used_names.insert(candidate.clone());
        self.name_count += 1;
        candidate
    }

    /// Simple hash function (FNV-1a inspired)
    fn simple_hash(&self, s: &str) -> u64 {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in s.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    /// Print summary of mappings
    fn print_summary(&self) {
        eprintln!("\nAnonymization complete:");
        eprintln!("  Explicit mappings: {}", self.explicit_maps.len());
        eprintln!("  Generated names: {}", self.generated_maps.len());
        eprintln!("  Total unique names: {}", self.used_names.len());
    }
}

fn anonymize_csv(
    input: &PathBuf,
    output: &PathBuf,
    key: &str,
    map: &str,
    columns: &[String],
) -> Result<()> {
    if key.is_empty() {
        return Err(anyhow::anyhow!(
            "Anonymization key is required. Set BBO_ANON_KEY env var or use --key"
        ));
    }

    // Read and preprocess input CSV to fix BBO's malformed quoting
    let csv_data = read_bbo_csv_fixed(input)?;
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_data.as_bytes());
    let headers = reader.headers()?.clone();

    // Find column indices to anonymize
    let col_indices: Vec<usize> = columns
        .iter()
        .filter_map(|col| headers.iter().position(|h| h == col))
        .collect();

    if col_indices.is_empty() {
        return Err(anyhow::anyhow!(
            "None of the specified columns ({}) found in CSV",
            columns.join(", ")
        ));
    }

    // Find LIN_URL column for special handling (contains embedded usernames)
    let lin_url_idx = headers.iter().position(|h| h == "LIN_URL");

    eprintln!(
        "Anonymizing columns: {:?}{}",
        col_indices
            .iter()
            .map(|&i| headers.get(i).unwrap_or("?"))
            .collect::<Vec<_>>(),
        if lin_url_idx.is_some() { " + LIN_URL (embedded names)" } else { "" }
    );

    // Create anonymizer
    let mut anonymizer = Anonymizer::new(key, map);

    // Count rows for progress
    let total_rows = count_csv_rows(input)?;

    // Open output
    let mut writer = Writer::from_path(output).context("Failed to create output CSV")?;
    writer.write_record(&headers)?;

    let mut processed = 0;

    for result in reader.records() {
        let record = result.context("Failed to read CSV row")?;
        processed += 1;

        if processed % 1000 == 0 {
            eprint!("\r[{}/{}] Anonymizing...    ", processed, total_rows);
            std::io::stderr().flush().ok();
        }

        // Build output record with anonymized columns
        let mut output_fields: Vec<String> = Vec::with_capacity(record.len());

        for (i, field) in record.iter().enumerate() {
            if col_indices.contains(&i) && !field.is_empty() {
                output_fields.push(anonymizer.anonymize(field));
            } else if Some(i) == lin_url_idx && !field.is_empty() {
                // Special handling for LIN_URL - anonymize embedded player names
                output_fields.push(anonymize_lin_url(field, &mut anonymizer));
            } else {
                output_fields.push(field.to_string());
            }
        }

        writer.write_record(&output_fields)?;
    }

    writer.flush()?;
    eprint!("\r[{}/{}] Anonymizing...    ", processed, total_rows);
    anonymizer.print_summary();

    Ok(())
}

/// Anonymize player names embedded in a BBO LIN URL
/// LIN URLs contain player names in pn| tags, which may be URL-encoded:
/// - Literal: pn|player1,player2,player3,player4|
/// - Encoded: pn%7Cplayer1%2Cplayer2%2Cplayer3%2Cplayer4%7C
fn anonymize_lin_url(url: &str, anonymizer: &mut Anonymizer) -> String {
    lazy_static::lazy_static! {
        // Match URL-encoded format: pn%7C...%7C (where %7C = | and names separated by %2C = ,)
        static ref PN_ENCODED: Regex = Regex::new(r"(?i)pn%7C([^%]+(?:%2C[^%]+)*)%7C").unwrap();
        // Match literal format: pn|...|
        static ref PN_LITERAL: Regex = Regex::new(r"pn\|([^|]+)\|").unwrap();
    }

    // Try URL-encoded format first (more common in BBO URLs)
    let result = PN_ENCODED.replace(url, |caps: &regex::Captures| {
        let names_str = &caps[1];
        let anon_names: Vec<String> = names_str
            .split("%2C")
            .map(|name| {
                let name = name.trim();
                if name.is_empty() {
                    String::new()
                } else {
                    anonymizer.anonymize(name)
                }
            })
            .collect();
        format!("pn%7C{}%7C", anon_names.join("%2C"))
    });

    // If no encoded match, try literal format
    let result = PN_LITERAL.replace(&result, |caps: &regex::Captures| {
        let names = &caps[1];
        let anon_names: Vec<String> = names
            .split(',')
            .map(|name| {
                let name = name.trim();
                if name.is_empty() {
                    String::new()
                } else {
                    anonymizer.anonymize(name)
                }
            })
            .collect();
        format!("pn|{}|", anon_names.join(","))
    });

    result.to_string()
}

// ============================================================================
// Display Hand Implementation
// ============================================================================

fn display_hand(input: &PathBuf, row_num: usize) -> Result<()> {
    if row_num == 0 {
        return Err(anyhow::anyhow!("Row number must be 1 or greater"));
    }

    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(input)
        .context("Failed to open input CSV")?;
    let headers = reader.headers()?.clone();

    // Find required columns
    let find_col = |name: &str| headers.iter().position(|h| h == name);

    let north_col = find_col("North hand").or_else(|| find_col("N hand"));
    let south_col = find_col("South hand").or_else(|| find_col("S hand"));
    let east_col = find_col("East hand").or_else(|| find_col("E hand"));
    let west_col = find_col("West hand").or_else(|| find_col("W hand"));
    let contract_col = find_col("Contract");
    let declarer_col = find_col("Dec");
    let result_col = find_col("Result");
    let cardplay_col = find_col("Cardplay");
    let dd_col = find_col("DD_Analysis");
    let n_col = find_col("N");
    let s_col = find_col("S");
    let e_col = find_col("E");
    let w_col = find_col("W");
    let ref_col = find_col("Ref #");

    // Skip to the requested row
    let record = reader
        .records()
        .nth(row_num - 1)
        .ok_or_else(|| anyhow::anyhow!("Row {} not found in file", row_num))?
        .context("Failed to read CSV row")?;

    // Extract data
    let get = |col: Option<usize>| col.and_then(|i| record.get(i)).unwrap_or("");

    let north_hand = get(north_col);
    let south_hand = get(south_col);
    let east_hand = get(east_col);
    let west_hand = get(west_col);
    let contract = get(contract_col);
    let declarer = get(declarer_col);
    let result = get(result_col);
    let cardplay = get(cardplay_col);
    let dd_analysis = get(dd_col);
    let north_player = get(n_col);
    let south_player = get(s_col);
    let east_player = get(e_col);
    let west_player = get(w_col);
    let ref_num = get(ref_col);

    // Print header
    println!("\n{:=^80}", format!(" Hand #{} (Ref: {}) ", row_num, ref_num));

    // Print contract info
    println!("\nContract: {} by {}    Result: {}", contract, declarer, result);
    println!("Players: N={} S={} E={} W={}", north_player, south_player, east_player, west_player);

    // Print the deal in a nice format
    println!("\n{:^80}", "DEAL");
    println!("{:-<80}", "");

    // Parse and display hands
    let format_suit = |hand: &str, suit_char: char| -> String {
        // Hand format: "S:AKQ H:JT9 D:876 C:5432" or similar
        for part in hand.split_whitespace() {
            let lower_suit = suit_char.to_ascii_lowercase();
            if part.starts_with(suit_char) || part.starts_with(lower_suit) {
                if let Some(cards) = part.get(2..) {
                    return cards.to_string();
                }
            }
        }
        "-".to_string()
    };

    let format_hand_lines = |hand: &str| -> [String; 4] {
        [
            format!("S: {}", format_suit(hand, 'S')),
            format!("H: {}", format_suit(hand, 'H')),
            format!("D: {}", format_suit(hand, 'D')),
            format!("C: {}", format_suit(hand, 'C')),
        ]
    };

    let north_lines = format_hand_lines(north_hand);
    let south_lines = format_hand_lines(south_hand);
    let east_lines = format_hand_lines(east_hand);
    let west_lines = format_hand_lines(west_hand);

    // Print North
    println!("{:^40}", "North");
    for line in &north_lines {
        println!("{:^40}", line);
    }

    // Print West and East side by side
    println!();
    println!("{:<20}{:>20}", "West", "East");
    for i in 0..4 {
        println!("{:<20}{:>20}", west_lines[i], east_lines[i]);
    }

    // Print South
    println!();
    println!("{:^40}", "South");
    for line in &south_lines {
        println!("{:^40}", line);
    }

    // Print cardplay
    println!("\n{:=^80}", " CARDPLAY ");

    if cardplay.is_empty() {
        println!("(No cardplay recorded)");
    } else {
        // Determine initial leader (left of declarer)
        let initial_leader = match declarer.chars().next() {
            Some('N') => 'E',
            Some('E') => 'S',
            Some('S') => 'W',
            Some('W') => 'N',
            _ => '?',
        };

        // Parse DD analysis into a map: trick_num -> costs
        let mut dd_costs: HashMap<usize, Vec<u8>> = HashMap::new();
        if !dd_analysis.is_empty() && !dd_analysis.starts_with("ERROR") {
            for trick_str in dd_analysis.split('|') {
                if let Some(colon_idx) = trick_str.find(':') {
                    let trick_num_str = &trick_str[1..colon_idx]; // Skip 'T'
                    if let Ok(trick_num) = trick_num_str.parse::<usize>() {
                        let costs: Vec<u8> = trick_str[colon_idx + 1..]
                            .split(',')
                            .filter_map(|s| s.trim().parse().ok())
                            .collect();
                        if costs.len() == 4 {
                            dd_costs.insert(trick_num, costs);
                        }
                    }
                }
            }
        }

        // Print header
        println!("\n{:>5} | {:^8} {:^8} {:^8} {:^8} | {:^20}",
            "Trick", "Leader", "2nd", "3rd", "4th", "DD Cost (L/2/3/4)");
        println!("{:-<80}", "");

        let mut current_leader = initial_leader;

        for (trick_idx, trick_str) in cardplay.split('|').enumerate() {
            if trick_str.is_empty() {
                continue;
            }

            let trick_num = trick_idx + 1;
            let cards: Vec<&str> = trick_str.split_whitespace().collect();

            if cards.len() != 4 {
                continue;
            }

            // Get seat order
            let seats = get_seat_order(current_leader);

            // Format cards with seats
            let card_strs: Vec<String> = cards
                .iter()
                .enumerate()
                .map(|(i, c)| format!("{}:{}", seats[i], c))
                .collect();

            // Get DD costs for this trick
            let costs = dd_costs.get(&trick_num);
            let cost_str = if let Some(c) = costs {
                format!("{},{},{},{}", c[0], c[1], c[2], c[3])
            } else {
                "-".to_string()
            };

            println!("{:>5} | {:^8} {:^8} {:^8} {:^8} | {:^20}",
                trick_num,
                card_strs.get(0).map(|s| s.as_str()).unwrap_or("-"),
                card_strs.get(1).map(|s| s.as_str()).unwrap_or("-"),
                card_strs.get(2).map(|s| s.as_str()).unwrap_or("-"),
                card_strs.get(3).map(|s| s.as_str()).unwrap_or("-"),
                cost_str
            );

            // Determine trick winner for next trick's leader
            // We need to look at the actual cards to determine the winner
            if let Some(winner_seat) = determine_trick_winner_for_display(&cards, current_leader, contract) {
                current_leader = winner_seat;
            }
        }
    }

    // Print DD Analysis summary
    if !dd_analysis.is_empty() && !dd_analysis.starts_with("ERROR") {
        println!("\n{:=^80}", " DD ANALYSIS SUMMARY ");

        // Count total cost by seat
        let mut seat_costs: HashMap<char, u64> = HashMap::new();
        let mut seat_plays: HashMap<char, u64> = HashMap::new();
        let mut seat_errors: HashMap<char, u64> = HashMap::new();

        let initial_leader = match declarer.chars().next() {
            Some('N') => 'E',
            Some('E') => 'S',
            Some('S') => 'W',
            Some('W') => 'N',
            _ => '?',
        };

        let tricks: Vec<&str> = cardplay.split('|').collect();
        let mut current_leader = initial_leader;

        for (trick_idx, trick_str) in dd_analysis.split('|').enumerate() {
            if let Some(colon_idx) = trick_str.find(':') {
                let costs: Vec<u8> = trick_str[colon_idx + 1..]
                    .split(',')
                    .filter_map(|s| s.trim().parse().ok())
                    .collect();

                if costs.len() == 4 {
                    let seats = get_seat_order(current_leader);
                    for (i, &cost) in costs.iter().enumerate() {
                        let seat = seats[i];
                        *seat_costs.entry(seat).or_insert(0) += cost as u64;
                        *seat_plays.entry(seat).or_insert(0) += 1;
                        if cost > 0 {
                            *seat_errors.entry(seat).or_insert(0) += 1;
                        }
                    }

                    // Determine next leader
                    if trick_idx < tricks.len() {
                        let cards: Vec<&str> = tricks[trick_idx].split_whitespace().collect();
                        if let Some(winner) = determine_trick_winner_for_display(&cards, current_leader, contract) {
                            current_leader = winner;
                        }
                    }
                }
            }
        }

        // Determine declaring side
        let declaring_seats: [char; 2] = match declarer.chars().next() {
            Some('N') | Some('S') => ['N', 'S'],
            Some('E') | Some('W') => ['E', 'W'],
            _ => ['?', '?'],
        };

        println!("\n{:<10} {:>10} {:>10} {:>12} {:>10}", "Seat", "Plays", "Errors", "Total Cost", "Role");
        println!("{:-<60}", "");

        for seat in ['N', 'E', 'S', 'W'] {
            let plays = seat_plays.get(&seat).unwrap_or(&0);
            let errors = seat_errors.get(&seat).unwrap_or(&0);
            let cost = seat_costs.get(&seat).unwrap_or(&0);
            let role = if declaring_seats.contains(&seat) { "Declaring" } else { "Defending" };

            println!("{:<10} {:>10} {:>10} {:>12} {:>10}", seat, plays, errors, cost, role);
        }
    } else if dd_analysis.starts_with("ERROR") {
        println!("\n{:=^80}", " DD ANALYSIS ");
        println!("Error: {}", dd_analysis);
    }

    println!("\n{:=^80}", "");

    Ok(())
}

/// Determine trick winner based on cards played (for display purposes)
fn determine_trick_winner_for_display(cards: &[&str], leader: char, contract: &str) -> Option<char> {
    if cards.len() != 4 {
        return None;
    }

    // Parse trump suit from contract
    let trump = if contract.contains('N') {
        None // NT
    } else if contract.contains('S') {
        Some('S')
    } else if contract.contains('H') {
        Some('H')
    } else if contract.contains('D') {
        Some('D')
    } else if contract.contains('C') {
        Some('C')
    } else {
        None
    };

    // Parse cards
    let parse_card = |s: &str| -> Option<(char, u8)> {
        let s = s.trim();
        if s.len() < 2 {
            return None;
        }
        let suit = s.chars().next()?;
        let rank_char = s.chars().nth(1)?;
        let rank = match rank_char {
            'A' => 14,
            'K' => 13,
            'Q' => 12,
            'J' => 11,
            'T' | '1' => 10,
            '9' => 9,
            '8' => 8,
            '7' => 7,
            '6' => 6,
            '5' => 5,
            '4' => 4,
            '3' => 3,
            '2' => 2,
            _ => return None,
        };
        Some((suit, rank))
    };

    let parsed: Vec<Option<(char, u8)>> = cards.iter().map(|c| parse_card(c)).collect();

    // Lead suit
    let lead_suit = parsed[0].map(|(s, _)| s)?;

    // Find winner
    let mut winner_idx = 0;
    let mut winning_card = parsed[0]?;

    for (i, card_opt) in parsed.iter().enumerate().skip(1) {
        if let Some((suit, rank)) = card_opt {
            let dominated = if let Some(t) = trump {
                // Trump beats non-trump
                if *suit == t && winning_card.0 != t {
                    true
                } else if *suit == t && winning_card.0 == t {
                    *rank > winning_card.1
                } else if winning_card.0 == t {
                    false
                } else if *suit == lead_suit {
                    *rank > winning_card.1
                } else {
                    false
                }
            } else {
                // No trump: must follow suit
                *suit == lead_suit && *rank > winning_card.1
            };

            if dominated {
                winner_idx = i;
                winning_card = (*suit, *rank);
            }
        }
    }

    // Map winner index to seat
    let seats = get_seat_order(leader);
    Some(seats[winner_idx])
}

// ============================================================================
// Stats Implementation
// ============================================================================

/// Statistics for a player
#[derive(Default, Clone)]
struct PlayerStats {
    name: String,
    // Declaring stats
    declaring_plays: u64,
    declaring_errors: u64,
    declaring_total_cost: u64,
    declaring_deals: u64,
    // Defending stats
    defending_plays: u64,
    defending_errors: u64,
    defending_total_cost: u64,
    defending_deals: u64,
}

impl PlayerStats {
    fn new(name: &str) -> Self {
        PlayerStats {
            name: name.to_string(),
            ..Default::default()
        }
    }

    fn declaring_error_rate(&self) -> f64 {
        if self.declaring_plays == 0 {
            0.0
        } else {
            self.declaring_errors as f64 / self.declaring_plays as f64 * 100.0
        }
    }

    fn defending_error_rate(&self) -> f64 {
        if self.defending_plays == 0 {
            0.0
        } else {
            self.defending_errors as f64 / self.defending_plays as f64 * 100.0
        }
    }

    fn declaring_avg_cost(&self) -> f64 {
        if self.declaring_plays == 0 {
            0.0
        } else {
            self.declaring_total_cost as f64 / self.declaring_plays as f64
        }
    }

    fn defending_avg_cost(&self) -> f64 {
        if self.defending_plays == 0 {
            0.0
        } else {
            self.defending_total_cost as f64 / self.defending_plays as f64
        }
    }

    fn total_deals(&self) -> u64 {
        // A player is in every deal as either declarer/dummy or defender
        // But we track deals separately, so take max to avoid double counting
        self.declaring_deals + self.defending_deals
    }

    /// Merge another player's stats into this one (for "Field" aggregation)
    fn merge(&mut self, other: &PlayerStats) {
        self.declaring_plays += other.declaring_plays;
        self.declaring_errors += other.declaring_errors;
        self.declaring_total_cost += other.declaring_total_cost;
        self.declaring_deals += other.declaring_deals;
        self.defending_plays += other.defending_plays;
        self.defending_errors += other.defending_errors;
        self.defending_total_cost += other.defending_total_cost;
        self.defending_deals += other.defending_deals;
    }

    /// 95% confidence interval half-width for error rate (using normal approximation)
    fn declaring_ci(&self) -> f64 {
        if self.declaring_plays < 30 {
            return f64::NAN;
        }
        let p = self.declaring_errors as f64 / self.declaring_plays as f64;
        let n = self.declaring_plays as f64;
        1.96 * (p * (1.0 - p) / n).sqrt() * 100.0
    }

    fn defending_ci(&self) -> f64 {
        if self.defending_plays < 30 {
            return f64::NAN;
        }
        let p = self.defending_errors as f64 / self.defending_plays as f64;
        let n = self.defending_plays as f64;
        1.96 * (p * (1.0 - p) / n).sqrt() * 100.0
    }

    /// Calculate the Def - Decl difference (expected to be positive for honest players)
    fn def_minus_decl(&self) -> f64 {
        self.defending_error_rate() - self.declaring_error_rate()
    }

    /// Standard error for the Def - Decl difference
    fn diff_se(&self) -> f64 {
        if self.declaring_plays < 30 || self.defending_plays < 30 {
            return f64::NAN;
        }
        let p1 = self.declaring_errors as f64 / self.declaring_plays as f64;
        let n1 = self.declaring_plays as f64;
        let p2 = self.defending_errors as f64 / self.defending_plays as f64;
        let n2 = self.defending_plays as f64;
        // SE of difference of two proportions
        ((p1 * (1.0 - p1) / n1) + (p2 * (1.0 - p2) / n2)).sqrt() * 100.0
    }
}

/// Z-test comparing two players' Def-Decl differences
/// Returns (z-score, p-value) for one-tailed test
fn z_test_diff_vs_baseline(subject: &PlayerStats, baseline: &PlayerStats) -> (f64, f64) {
    let diff_subj = subject.def_minus_decl();
    let diff_base = baseline.def_minus_decl();

    let se_subj = subject.diff_se();
    let se_base = baseline.diff_se();

    if se_subj.is_nan() || se_base.is_nan() {
        return (f64::NAN, f64::NAN);
    }

    // Combined SE for comparing two differences
    let se_combined = (se_subj.powi(2) + se_base.powi(2)).sqrt();

    // Z-score: how many SEs is subject's diff below baseline's diff?
    let z = (diff_subj - diff_base) / se_combined;

    // One-tailed p-value (testing if subject's diff is significantly LOWER)
    // Using standard normal approximation
    let p = 0.5 * (1.0 + erf(-z / std::f64::consts::SQRT_2));

    (z, p)
}

/// Error function approximation (for p-value calculation)
fn erf(x: f64) -> f64 {
    // Horner form coefficients for erf approximation
    let a1 =  0.254829592;
    let a2 = -0.284496736;
    let a3 =  1.421413741;
    let a4 = -1.453152027;
    let a5 =  1.061405429;
    let p  =  0.3275911;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
    sign * y
}

fn compute_stats(input: &PathBuf, top_n: usize, output: Option<&PathBuf>) -> Result<()> {
    // Read input CSV
    let mut reader = ReaderBuilder::new()
        .flexible(true)
        .from_path(input)
        .context("Failed to open input CSV")?;
    let headers = reader.headers()?.clone();

    // Find required columns
    let n_col = headers.iter().position(|h| h == "N")
        .ok_or_else(|| anyhow::anyhow!("Column 'N' not found"))?;
    let s_col = headers.iter().position(|h| h == "S")
        .ok_or_else(|| anyhow::anyhow!("Column 'S' not found"))?;
    let e_col = headers.iter().position(|h| h == "E")
        .ok_or_else(|| anyhow::anyhow!("Column 'E' not found"))?;
    let w_col = headers.iter().position(|h| h == "W")
        .ok_or_else(|| anyhow::anyhow!("Column 'W' not found"))?;
    let dec_col = headers.iter().position(|h| h == "Dec")
        .ok_or_else(|| anyhow::anyhow!("Column 'Dec' not found"))?;
    let dd_col = headers.iter().position(|h| h == "DD_Analysis")
        .ok_or_else(|| anyhow::anyhow!("Column 'DD_Analysis' not found - run analyze-dd first"))?;

    // Collect stats per player
    let mut player_stats: HashMap<String, PlayerStats> = HashMap::new();
    let mut processed = 0;
    let mut skipped = 0;

    for result in reader.records() {
        let record = result.context("Failed to read CSV row")?;
        processed += 1;

        // Get player names
        let north = record.get(n_col).unwrap_or("").to_lowercase();
        let south = record.get(s_col).unwrap_or("").to_lowercase();
        let east = record.get(e_col).unwrap_or("").to_lowercase();
        let west = record.get(w_col).unwrap_or("").to_lowercase();

        // Get declarer
        let declarer = record.get(dec_col).unwrap_or("").trim().to_uppercase();
        if declarer.is_empty() {
            skipped += 1;
            continue;
        }

        // Get DD analysis
        let dd_analysis = record.get(dd_col).unwrap_or("");
        if dd_analysis.is_empty() || dd_analysis.starts_with("ERROR") {
            skipped += 1;
            continue;
        }

        // Determine declarer and dummy
        let (declarer_name, dummy_name, def1_name, def2_name) = match declarer.chars().next() {
            Some('N') => (&north, &south, &east, &west),
            Some('S') => (&south, &north, &east, &west),
            Some('E') => (&east, &west, &north, &south),
            Some('W') => (&west, &east, &north, &south),
            _ => {
                skipped += 1;
                continue;
            }
        };

        // Parse DD analysis and attribute costs
        // Format: T1:c1,c2,c3,c4|T2:c1,c2,c3,c4|...
        // Order in each trick: leader, 2nd, 3rd, 4th
        // Leader of trick 1 is left of declarer

        let initial_leader = match declarer.chars().next() {
            Some('N') => 'E', // E leads vs N
            Some('E') => 'S', // S leads vs E
            Some('S') => 'W', // W leads vs S
            Some('W') => 'N', // N leads vs W
            _ => continue,
        };

        // Track who made each play and their cost
        let mut current_leader = initial_leader;

        for trick_str in dd_analysis.split('|') {
            // Parse "T1:c1,c2,c3,c4"
            let costs_part = if let Some(idx) = trick_str.find(':') {
                &trick_str[idx + 1..]
            } else {
                continue;
            };

            let costs: Vec<u8> = costs_part
                .split(',')
                .filter_map(|s| s.trim().parse().ok())
                .collect();

            if costs.len() != 4 {
                continue;
            }

            // Determine seat order for this trick
            let seat_order = get_seat_order(current_leader);

            // Attribute each cost to a player
            let mut trick_winner = current_leader;
            let mut max_in_trick = 0u8; // We don't track suit, so just track position

            for (i, &cost) in costs.iter().enumerate() {
                let seat = seat_order[i];
                let player_name = match seat {
                    'N' => &north,
                    'S' => &south,
                    'E' => &east,
                    'W' => &west,
                    _ => continue,
                };

                if player_name.is_empty() {
                    continue;
                }

                let stats = player_stats
                    .entry(player_name.clone())
                    .or_insert_with(|| PlayerStats::new(player_name));

                // Is this player declaring/dummy or defending?
                let is_declaring_side = player_name == declarer_name || player_name == dummy_name;

                if is_declaring_side {
                    stats.declaring_plays += 1;
                    if cost > 0 {
                        stats.declaring_errors += 1;
                    }
                    stats.declaring_total_cost += cost as u64;
                } else {
                    stats.defending_plays += 1;
                    if cost > 0 {
                        stats.defending_errors += 1;
                    }
                    stats.defending_total_cost += cost as u64;
                }

                // Simple trick winner tracking (position 0 wins ties)
                if i == 0 || cost == 0 {
                    if i == 0 {
                        trick_winner = seat;
                        max_in_trick = 0;
                    }
                }
            }

            // For simplicity, rotate leader clockwise (we don't have actual trick winner info here)
            // This is approximate - a better approach would track actual cards
            current_leader = trick_winner; // This is a rough approximation
        }

        // Track deals per player
        for name in [&north, &south, &east, &west] {
            if name.is_empty() {
                continue;
            }
            let stats = player_stats
                .entry(name.clone())
                .or_insert_with(|| PlayerStats::new(name));

            let is_declaring_side = name == declarer_name || name == dummy_name;
            if is_declaring_side {
                stats.declaring_deals += 1;
            } else {
                stats.defending_deals += 1;
            }
        }
    }

    eprintln!("Processed {} deals ({} skipped)", processed, skipped);
    eprintln!("Found {} unique players\n", player_stats.len());

    // Sort players by total deals (frequency)
    let mut players: Vec<_> = player_stats.values().cloned().collect();
    players.sort_by(|a, b| b.total_deals().cmp(&a.total_deals()));

    // Identify top 2 players (the subjects)
    let top_2: HashSet<String> = players.iter().take(2).map(|p| p.name.clone()).collect();

    // Create "Field" by aggregating everyone except top 2
    let mut field_stats = PlayerStats::new("FIELD");
    for player in &players {
        if !top_2.contains(&player.name) {
            field_stats.merge(player);
        }
    }

    // Print header
    println!("\n{:=^100}", " DD Error Rate Analysis ");
    println!("\n{:<20} {:>8} {:>12} {:>10} {:>12} {:>10} {:>10}",
        "Player", "Deals", "Decl Plays", "Decl Err%", "Def Plays", "Def Err%", "Diff");
    println!("{:-<100}", "");

    // Print top N players
    for player in players.iter().take(top_n) {
        let decl_rate = player.declaring_error_rate();
        let def_rate = player.defending_error_rate();
        let diff = decl_rate - def_rate;
        let decl_ci = player.declaring_ci();
        let def_ci = player.defending_ci();

        println!("{:<20} {:>8} {:>12} {:>9.2}% {:>12} {:>9.2}% {:>+9.2}%",
            truncate_name(&player.name, 20),
            player.total_deals(),
            player.declaring_plays,
            decl_rate,
            player.defending_plays,
            def_rate,
            diff
        );

        // Print confidence intervals on separate line if enough data
        if !decl_ci.is_nan() || !def_ci.is_nan() {
            println!("{:<20} {:>8} {:>12} {:>10} {:>12} {:>10}",
                "",
                "",
                format!("(±{:.2}%)", decl_ci),
                "",
                format!("(±{:.2}%)", def_ci),
                ""
            );
        }
    }

    // Print Field aggregate
    println!("{:-<100}", "");
    let decl_rate = field_stats.declaring_error_rate();
    let def_rate = field_stats.defending_error_rate();
    let diff = decl_rate - def_rate;

    println!("{:<20} {:>8} {:>12} {:>9.2}% {:>12} {:>9.2}% {:>+9.2}%",
        "FIELD (others)",
        field_stats.total_deals(),
        field_stats.declaring_plays,
        decl_rate,
        field_stats.defending_plays,
        def_rate,
        diff
    );
    println!("{:<20} {:>8} {:>12} {:>10} {:>12} {:>10}",
        "",
        "",
        format!("(±{:.2}%)", field_stats.declaring_ci()),
        "",
        format!("(±{:.2}%)", field_stats.defending_ci()),
        ""
    );

    // Partner Comparison Section (if we have at least 2 players)
    if players.len() >= 2 {
        let subj_a = &players[0];
        let subj_b = &players[1];

        println!("\n{:=^100}", " Partner Comparison ");
        println!("\nComparing {} vs {}:", subj_a.name, subj_b.name);

        // Declaring comparison
        let decl_gap = subj_a.declaring_error_rate() - subj_b.declaring_error_rate();
        println!("\n  DECLARING:");
        println!("    {:<20}: {:.2}% error rate", subj_a.name, subj_a.declaring_error_rate());
        println!("    {:<20}: {:.2}% error rate", subj_b.name, subj_b.declaring_error_rate());
        println!("    Skill gap: {:+.2}% ({} has {} errors declaring)",
            decl_gap,
            if decl_gap > 0.0 { &subj_a.name } else { &subj_b.name },
            "more"
        );

        // Defending comparison
        let def_gap = subj_a.defending_error_rate() - subj_b.defending_error_rate();
        println!("\n  DEFENDING:");
        println!("    {:<20}: {:.2}% error rate", subj_a.name, subj_a.defending_error_rate());
        println!("    {:<20}: {:.2}% error rate", subj_b.name, subj_b.defending_error_rate());
        println!("    Skill gap: {:+.2}% ({} has {} errors defending)",
            def_gap,
            if def_gap > 0.0 { &subj_a.name } else { &subj_b.name },
            "more"
        );

        // Convergence analysis
        println!("\n  CONVERGENCE ANALYSIS:");
        let convergence = decl_gap.abs() - def_gap.abs();
        if convergence > 1.0 {
            println!("    ⚠️  Skill gap NARROWS by {:.2}% on defense (declaring gap: {:.2}%, defense gap: {:.2}%)",
                convergence, decl_gap.abs(), def_gap.abs());
            println!("    This pattern (partners performing more similarly on defense) can indicate hand sharing.");
        } else if convergence < -1.0 {
            println!("    Skill gap WIDENS by {:.2}% on defense - consistent with honest play",
                -convergence);
        } else {
            println!("    Skill gap is similar in both roles ({:.2}% declaring, {:.2}% defending)",
                decl_gap.abs(), def_gap.abs());
        }

        // Statistical Test Section
        println!("\n{:=^100}", " Statistical Analysis ");

        // Compare each subject to Field baseline
        for subj in [subj_a, subj_b] {
            let subj_diff = subj.def_minus_decl();
            let field_diff = field_stats.def_minus_decl();
            let (z, p) = z_test_diff_vs_baseline(subj, &field_stats);

            println!("\n  {} vs FIELD baseline:", subj.name);
            println!("    {} Def-Decl diff: {:+.2}%", subj.name, subj_diff);
            println!("    FIELD Def-Decl diff:      {:+.2}%", field_diff);

            if !z.is_nan() {
                println!("    Z-score: {:.2}", z);
                if p < 0.001 {
                    println!("    P-value: <0.001 (highly significant)");
                } else if p < 0.01 {
                    println!("    P-value: {:.4} (significant at 1%)", p);
                } else if p < 0.05 {
                    println!("    P-value: {:.4} (significant at 5%)", p);
                } else {
                    println!("    P-value: {:.4} (not statistically significant)", p);
                }

                if z < -1.96 {
                    println!("    ⚠️  {}'s defense error rate is SUSPICIOUSLY LOW relative to their declaring rate", subj.name);
                } else if z > 1.96 {
                    println!("    ✓ {}'s pattern is NORMAL - defense errors exceed declaring as expected", subj.name);
                } else {
                    println!("    Results inconclusive - need more data for reliable inference");
                }
            } else {
                println!("    (Insufficient data for statistical test)");
            }
        }
    }

    println!("\n{:=^100}", "");
    println!("\nInterpretation:");
    println!("  - Decl Err%: Percentage of plays with DD cost > 0 when declaring/dummy");
    println!("  - Def Err%:  Percentage of plays with DD cost > 0 when defending");
    println!("  - Diff:      Decl% - Def% (negative means more errors on defense)");
    println!("\n  EXPECTED for honest players:");
    println!("    Defense is HARDER than declaring (defender sees fewer cards)");
    println!("    So honest players typically have MORE errors on defense (negative Diff)");
    println!("    The FIELD baseline shows the typical Def-Decl difference");
    println!("\n  RED FLAGS for potential hand-sharing:");
    println!("    - Defense error rate LOWER than declaring (positive Diff)");
    println!("    - Def-Decl pattern significantly different from FIELD");
    println!("    - Partners' skill gap narrowing on defense vs declaring");

    // Write detailed CSV if requested
    if let Some(output_path) = output {
        let mut writer = Writer::from_path(output_path)
            .context("Failed to create output CSV")?;

        writer.write_record(&[
            "Player", "Total_Deals", "Decl_Deals", "Def_Deals",
            "Decl_Plays", "Decl_Errors", "Decl_Err_Pct", "Decl_Avg_Cost", "Decl_CI",
            "Def_Plays", "Def_Errors", "Def_Err_Pct", "Def_Avg_Cost", "Def_CI",
            "Diff_Pct"
        ])?;

        for player in &players {
            writer.write_record(&[
                &player.name,
                &player.total_deals().to_string(),
                &player.declaring_deals.to_string(),
                &player.defending_deals.to_string(),
                &player.declaring_plays.to_string(),
                &player.declaring_errors.to_string(),
                &format!("{:.4}", player.declaring_error_rate()),
                &format!("{:.4}", player.declaring_avg_cost()),
                &format!("{:.4}", player.declaring_ci()),
                &player.defending_plays.to_string(),
                &player.defending_errors.to_string(),
                &format!("{:.4}", player.defending_error_rate()),
                &format!("{:.4}", player.defending_avg_cost()),
                &format!("{:.4}", player.defending_ci()),
                &format!("{:.4}", player.declaring_error_rate() - player.defending_error_rate()),
            ])?;
        }

        // Add Field row
        writer.write_record(&[
            "FIELD",
            &field_stats.total_deals().to_string(),
            &field_stats.declaring_deals.to_string(),
            &field_stats.defending_deals.to_string(),
            &field_stats.declaring_plays.to_string(),
            &field_stats.declaring_errors.to_string(),
            &format!("{:.4}", field_stats.declaring_error_rate()),
            &format!("{:.4}", field_stats.declaring_avg_cost()),
            &format!("{:.4}", field_stats.declaring_ci()),
            &field_stats.defending_plays.to_string(),
            &field_stats.defending_errors.to_string(),
            &format!("{:.4}", field_stats.defending_error_rate()),
            &format!("{:.4}", field_stats.defending_avg_cost()),
            &format!("{:.4}", field_stats.defending_ci()),
            &format!("{:.4}", field_stats.declaring_error_rate() - field_stats.defending_error_rate()),
        ])?;

        writer.flush()?;
        eprintln!("\nDetailed stats written to: {}", output_path.display());
    }

    Ok(())
}

/// Get seat order starting from leader going clockwise
fn get_seat_order(leader: char) -> [char; 4] {
    match leader {
        'N' => ['N', 'E', 'S', 'W'],
        'E' => ['E', 'S', 'W', 'N'],
        'S' => ['S', 'W', 'N', 'E'],
        'W' => ['W', 'N', 'E', 'S'],
        _ => ['N', 'E', 'S', 'W'],
    }
}

/// Truncate a name to fit in a column
fn truncate_name(name: &str, max_len: usize) -> String {
    if name.len() <= max_len {
        name.to_string()
    } else {
        format!("{}...", &name[..max_len - 3])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_trump() {
        assert_eq!(parse_trump("4S").unwrap(), SPADE);
        assert_eq!(parse_trump("3NT").unwrap(), NOTRUMP);
        assert_eq!(parse_trump("6H").unwrap(), HEART);
        assert_eq!(parse_trump("2D").unwrap(), DIAMOND);
        assert_eq!(parse_trump("5C").unwrap(), CLUB);
    }

    #[test]
    fn test_parse_declarer() {
        assert_eq!(parse_declarer("N").unwrap(), NORTH);
        assert_eq!(parse_declarer("E").unwrap(), EAST);
        assert_eq!(parse_declarer("S").unwrap(), SOUTH);
        assert_eq!(parse_declarer("W").unwrap(), WEST);
    }

    #[test]
    fn test_parse_cardplay() {
        let tricks = parse_cardplay("D2 DA D6 D5|S3 S2 SQ SA").unwrap();
        assert_eq!(tricks.len(), 2);
        assert_eq!(tricks[0].len(), 4);
        assert_eq!(tricks[1].len(), 4);
    }
}
