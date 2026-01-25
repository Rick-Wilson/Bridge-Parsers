use crate::error::Result;
use crate::{Board, Contract, Direction, Hand, Rank, Suit, Vulnerability, calculate_matchpoints};
use rust_xlsxwriter::{
    ConditionalFormat3ColorScale, Format, FormatAlign, FormatBorder, Workbook, Worksheet,
};
use std::collections::HashMap;
use std::path::Path;

/// Write boards to an Excel file
pub fn write_boards_to_xlsx(boards: &[Board], path: &Path) -> Result<()> {
    let mut workbook = Workbook::new();

    // Add the hand records worksheet
    let worksheet = workbook.add_worksheet();
    write_hand_records_sheet(worksheet, boards)?;

    workbook.save(path)?;
    Ok(())
}

/// Write hand records to a worksheet
fn write_hand_records_sheet(sheet: &mut Worksheet, boards: &[Board]) -> Result<()> {
    // Set column widths
    sheet.set_column_width(0, 8)?;   // Board
    sheet.set_column_width(1, 8)?;   // Dealer
    sheet.set_column_width(2, 10)?;  // Vul
    sheet.set_column_width(3, 14)?;  // North
    sheet.set_column_width(4, 14)?;  // East
    sheet.set_column_width(5, 14)?;  // South
    sheet.set_column_width(6, 14)?;  // West
    sheet.set_column_width(7, 6)?;   // N HCP
    sheet.set_column_width(8, 6)?;   // E HCP
    sheet.set_column_width(9, 6)?;   // S HCP
    sheet.set_column_width(10, 6)?;  // W HCP
    sheet.set_column_width(11, 24)?; // DD Tricks
    sheet.set_column_width(12, 12)?; // Optimum Score
    sheet.set_column_width(13, 14)?; // Par Contract

    // Header format
    let header_format = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_border_bottom(FormatBorder::Thin);

    // Write headers
    let headers = [
        "Board", "Dealer", "Vul",
        "North", "East", "South", "West",
        "N HCP", "E HCP", "S HCP", "W HCP",
        "DD Tricks", "Optimum", "Par"
    ];

    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(0, col as u16, *header, &header_format)?;
    }

    // Data format
    let center_format = Format::new().set_align(FormatAlign::Center);
    let left_format = Format::new().set_align(FormatAlign::Left);

    // Write board data
    for (row_idx, board) in boards.iter().enumerate() {
        let row = (row_idx + 1) as u32;

        // Board number
        if let Some(num) = board.number {
            sheet.write_number_with_format(row, 0, num as f64, &center_format)?;
        }

        // Dealer
        if let Some(dealer) = board.dealer {
            sheet.write_string_with_format(row, 1, &dealer.to_char().to_string(), &center_format)?;
        }

        // Vulnerability
        sheet.write_string_with_format(row, 2, board.vulnerable.to_pbn(), &center_format)?;

        // Hands - format as compact notation
        for (col_offset, dir) in [(3, Direction::North), (4, Direction::East), (5, Direction::South), (6, Direction::West)] {
            let hand = board.deal.hand(dir);
            let hand_str = format_hand_compact(hand);
            sheet.write_string_with_format(row, col_offset, &hand_str, &left_format)?;
        }

        // HCP values
        let hcp = board.all_hcp();
        for (col_offset, hcp_val) in [(7, hcp[0]), (8, hcp[1]), (9, hcp[2]), (10, hcp[3])] {
            sheet.write_number_with_format(row, col_offset, hcp_val as f64, &center_format)?;
        }

        // Double Dummy Tricks
        if let Some(ref dd) = board.double_dummy_tricks {
            sheet.write_string_with_format(row, 11, dd, &center_format)?;
        }

        // Optimum Score
        if let Some(ref opt) = board.optimum_score {
            sheet.write_string_with_format(row, 12, opt, &center_format)?;
        }

        // Par Contract
        if let Some(ref par) = board.par_contract {
            sheet.write_string_with_format(row, 13, par, &center_format)?;
        }
    }

    // Set worksheet name
    sheet.set_name("Hand Records")?;

    Ok(())
}

/// Format a hand in compact notation (S:AKQ H:JT9 D:876 C:5432)
fn format_hand_compact(hand: &Hand) -> String {
    let mut parts = Vec::new();

    for suit in Suit::ALL {
        let mut ranks: Vec<Rank> = hand.cards()
            .iter()
            .filter(|c| c.suit == suit)
            .map(|c| c.rank)
            .collect();
        ranks.sort_by(|a, b| b.cmp(a)); // Sort descending (Ace first)

        if !ranks.is_empty() {
            let ranks_str: String = ranks.iter().map(|r| r.to_char()).collect();
            parts.push(format!("{}{}", suit.to_char(), ranks_str));
        }
    }

    if parts.is_empty() {
        "---".to_string()
    } else {
        parts.join(" ")
    }
}

/// Pair matchpoint summary
#[derive(Debug, Default, Clone)]
struct PairMatchpoints {
    boards_played: u32,
    total_mp_pct: f64,  // Sum of matchpoint percentages
}

/// Calculate matchpoints for all results in BwsData
/// Returns: (per-result matchpoints, per-pair totals)
/// Pair key is (section, pair_number, is_ns)
fn calculate_all_matchpoints(data: &crate::bws::BwsData) -> (Vec<Option<f64>>, HashMap<(i32, i32, bool), PairMatchpoints>) {
    let results = &data.received_data;

    // Calculate scores for all results
    let scores: Vec<Option<i32>> = results.iter()
        .map(|r| calculate_score_for_result(r))
        .collect();

    // Group results by board for matchpoint calculation
    let mut board_results: HashMap<i32, Vec<(usize, i32)>> = HashMap::new();
    for (idx, result) in results.iter().enumerate() {
        if let Some(score) = scores[idx] {
            board_results.entry(result.board)
                .or_default()
                .push((idx, score));
        }
    }

    // Calculate matchpoints for each board
    let mut matchpoints: Vec<Option<f64>> = vec![None; results.len()];
    for (_board, board_scores) in &board_results {
        let ns_scores: Vec<i32> = board_scores.iter().map(|(_, s)| *s).collect();
        let mps = calculate_matchpoints(&ns_scores);
        for (i, (idx, _)) in board_scores.iter().enumerate() {
            matchpoints[*idx] = Some(mps[i]);
        }
    }

    // Aggregate matchpoints per pair
    // In a Mitchell movement, pair_ns is the NS pair number and pair_ew is the EW pair number
    let mut pair_totals: HashMap<(i32, i32, bool), PairMatchpoints> = HashMap::new();

    for (idx, result) in results.iter().enumerate() {
        if let Some(mp) = matchpoints[idx] {
            // NS pair gets the NS matchpoints
            let ns_key = (result.section, result.pair_ns, true);
            let ns_entry = pair_totals.entry(ns_key).or_default();
            ns_entry.boards_played += 1;
            ns_entry.total_mp_pct += mp;

            // EW pair gets the EW matchpoints (100 - NS)
            let ew_key = (result.section, result.pair_ew, false);
            let ew_entry = pair_totals.entry(ew_key).or_default();
            ew_entry.boards_played += 1;
            ew_entry.total_mp_pct += 100.0 - mp;
        }
    }

    (matchpoints, pair_totals)
}

/// Write BWS data to an Excel file
pub fn write_bws_to_xlsx(data: &crate::bws::BwsData, path: &Path) -> Result<()> {
    write_bws_to_xlsx_with_masterpoints(data, path, None)
}

/// Write BWS data to an Excel file with optional masterpoint data
pub fn write_bws_to_xlsx_with_masterpoints(
    data: &crate::bws::BwsData,
    path: &Path,
    member_data: Option<&HashMap<String, crate::acbl::MemberInfo>>,
) -> Result<()> {
    let mut workbook = Workbook::new();

    // Calculate matchpoints once for use in multiple sheets
    let (matchpoints, pair_totals) = calculate_all_matchpoints(data);

    // Add Game Results sheet
    let results_sheet = workbook.add_worksheet();
    write_game_results_sheet(results_sheet, data, &matchpoints)?;

    // Add Players sheet with matchpoint totals
    let players_sheet = workbook.add_worksheet();
    write_players_sheet(players_sheet, data, &pair_totals, member_data)?;

    // Add Sections sheet if there are sections
    if !data.sections.is_empty() {
        let sections_sheet = workbook.add_worksheet();
        write_sections_sheet(sections_sheet, data)?;
    }

    // Add Hand Records sheet if available
    if !data.boards.is_empty() {
        let hands_sheet = workbook.add_worksheet();
        write_hand_records_sheet(hands_sheet, &data.boards)?;
    }

    workbook.save(path)?;
    Ok(())
}

/// Calculate score for a result row
fn calculate_score_for_result(result: &crate::bws::tables::ReceivedDataRow) -> Option<i32> {
    let contract = Contract::parse(&result.contract)?;
    let tricks_relative = Contract::parse_result(&result.result)?;

    // Determine vulnerability from board number
    let board_num = result.board as u32;
    let vul = Vulnerability::from_board_number(board_num);

    // Check if declarer is vulnerable
    let declarer_dir = match result.ns_ew.as_str() {
        "N" => Direction::North,
        "S" => Direction::South,
        "E" => Direction::East,
        "W" => Direction::West,
        _ => return None,
    };
    let declarer_vul = vul.is_vulnerable(declarer_dir);

    let score = contract.score(tricks_relative, declarer_vul);

    // Return score from NS perspective
    Some(match result.ns_ew.as_str() {
        "N" | "S" => score,
        "E" | "W" => -score,
        _ => score,
    })
}

/// Write game results to a worksheet
fn write_game_results_sheet(
    sheet: &mut Worksheet,
    data: &crate::bws::BwsData,
    matchpoints: &[Option<f64>],
) -> Result<()> {
    sheet.set_name("Game Results")?;

    // Set column widths
    sheet.set_column_width(0, 8)?;   // Board
    sheet.set_column_width(1, 8)?;   // Section
    sheet.set_column_width(2, 6)?;   // Table
    sheet.set_column_width(3, 6)?;   // Round
    sheet.set_column_width(4, 8)?;   // NS Pair
    sheet.set_column_width(5, 8)?;   // EW Pair
    sheet.set_column_width(6, 10)?;  // Declarer
    sheet.set_column_width(7, 10)?;  // Contract
    sheet.set_column_width(8, 8)?;   // Result
    sheet.set_column_width(9, 10)?;  // Lead Card
    sheet.set_column_width(10, 8)?;  // Score
    sheet.set_column_width(11, 8)?;  // NS MP%
    sheet.set_column_width(12, 8)?;  // EW MP%

    // Header format
    let header_format = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_border_bottom(FormatBorder::Thin);

    // Write headers
    let headers = [
        "Board", "Section", "Table", "Round",
        "NS Pair", "EW Pair", "Declarer", "Contract", "Result", "Lead",
        "Score", "NS MP%", "EW MP%"
    ];

    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(0, col as u16, *header, &header_format)?;
    }

    // Data formats
    let center_format = Format::new().set_align(FormatAlign::Center);
    let score_format = Format::new().set_align(FormatAlign::Right);
    let mp_format = Format::new().set_align(FormatAlign::Right).set_num_format("0.0");

    // Calculate scores for all results
    let scores: Vec<Option<i32>> = data.received_data.iter()
        .map(|r| calculate_score_for_result(r))
        .collect();

    // Write result data (in original order to match matchpoints indices)
    for (row_idx, result) in data.received_data.iter().enumerate() {
        let row = (row_idx + 1) as u32;

        sheet.write_number_with_format(row, 0, result.board as f64, &center_format)?;
        sheet.write_number_with_format(row, 1, result.section as f64, &center_format)?;
        sheet.write_number_with_format(row, 2, result.table as f64, &center_format)?;
        sheet.write_number_with_format(row, 3, result.round as f64, &center_format)?;
        sheet.write_number_with_format(row, 4, result.pair_ns as f64, &center_format)?;
        sheet.write_number_with_format(row, 5, result.pair_ew as f64, &center_format)?;

        // Declarer direction
        let declarer_dir = match result.ns_ew.as_str() {
            "N" => "North",
            "S" => "South",
            "E" => "East",
            "W" => "West",
            _ => &result.ns_ew,
        };
        sheet.write_string_with_format(row, 6, declarer_dir, &center_format)?;

        sheet.write_string_with_format(row, 7, &result.contract, &center_format)?;
        sheet.write_string_with_format(row, 8, &result.result, &center_format)?;

        if let Some(ref lead) = result.lead_card {
            sheet.write_string_with_format(row, 9, lead, &center_format)?;
        }

        // Score (from NS perspective)
        if let Some(score) = scores[row_idx] {
            sheet.write_number_with_format(row, 10, score as f64, &score_format)?;
        }

        // Matchpoints
        if let Some(mp) = matchpoints[row_idx] {
            sheet.write_number_with_format(row, 11, mp, &mp_format)?;
            sheet.write_number_with_format(row, 12, 100.0 - mp, &mp_format)?;
        }
    }

    Ok(())
}

/// Write players to a worksheet (from PlayerNumbers - actual game participants)
/// Includes matchpoint totals and percentages per pair, plus ACBL masterpoints if available
fn write_players_sheet(
    sheet: &mut Worksheet,
    data: &crate::bws::BwsData,
    pair_totals: &HashMap<(i32, i32, bool), PairMatchpoints>,
    member_data: Option<&HashMap<String, crate::acbl::MemberInfo>>,
) -> Result<()> {
    sheet.set_name("Players")?;

    let has_masterpoints = member_data.is_some();

    // Set column widths
    sheet.set_column_width(0, 10)?;  // Section
    sheet.set_column_width(1, 6)?;   // Table
    sheet.set_column_width(2, 10)?;  // Direction
    sheet.set_column_width(3, 12)?;  // Player ID
    sheet.set_column_width(4, 25)?;  // Name
    sheet.set_column_width(5, 8)?;   // Boards
    sheet.set_column_width(6, 10)?;  // Total MP%
    sheet.set_column_width(7, 10)?;  // Avg MP%

    if has_masterpoints {
        sheet.set_column_width(8, 18)?;   // ACBL Rank
        sheet.set_column_width(9, 12)?;   // ACBL Points
    }

    // Header format
    let header_format = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_border_bottom(FormatBorder::Thin);

    let center_format = Format::new().set_align(FormatAlign::Center);
    let left_format = Format::new().set_align(FormatAlign::Left);
    let mp_format = Format::new().set_align(FormatAlign::Right).set_num_format("0.00");
    let points_format = Format::new().set_align(FormatAlign::Right).set_num_format("#,##0.00");

    // Write headers
    sheet.write_string_with_format(0, 0, "Section", &header_format)?;
    sheet.write_string_with_format(0, 1, "Table", &header_format)?;
    sheet.write_string_with_format(0, 2, "Direction", &header_format)?;
    sheet.write_string_with_format(0, 3, "Player ID", &header_format)?;
    sheet.write_string_with_format(0, 4, "Name", &header_format)?;
    sheet.write_string_with_format(0, 5, "Boards", &header_format)?;
    sheet.write_string_with_format(0, 6, "Total MP%", &header_format)?;
    sheet.write_string_with_format(0, 7, "Avg MP%", &header_format)?;

    if has_masterpoints {
        sheet.write_string_with_format(0, 8, "ACBL Rank", &header_format)?;
        sheet.write_string_with_format(0, 9, "ACBL Points", &header_format)?;
    }

    // Sort players by section, table, direction order (N, E, S, W)
    let mut players: Vec<_> = data.player_numbers.iter().collect();
    players.sort_by(|a, b| {
        a.section.cmp(&b.section)
            .then(a.table.cmp(&b.table))
            .then(direction_order(&a.direction).cmp(&direction_order(&b.direction)))
    });

    // Write player data
    for (row_idx, player) in players.iter().enumerate() {
        let row = (row_idx + 1) as u32;

        sheet.write_number_with_format(row, 0, player.section as f64, &center_format)?;
        sheet.write_number_with_format(row, 1, player.table as f64, &center_format)?;
        sheet.write_string_with_format(row, 2, &player.direction, &center_format)?;
        sheet.write_string_with_format(row, 3, &player.number, &left_format)?;
        if let Some(ref name) = player.name {
            sheet.write_string_with_format(row, 4, name, &left_format)?;
        }

        // Look up pair matchpoints
        // Pair is identified by (section, table, is_ns)
        // For the initial seating, table number = pair number
        let is_ns = player.direction == "N" || player.direction == "S";
        let pair_key = (player.section, player.table, is_ns);

        if let Some(mp_data) = pair_totals.get(&pair_key) {
            sheet.write_number_with_format(row, 5, mp_data.boards_played as f64, &center_format)?;
            sheet.write_number_with_format(row, 6, mp_data.total_mp_pct, &mp_format)?;

            // Average matchpoint percentage
            if mp_data.boards_played > 0 {
                let avg = mp_data.total_mp_pct / mp_data.boards_played as f64;
                sheet.write_number_with_format(row, 7, avg, &mp_format)?;
            }
        }

        // Look up ACBL masterpoint data if available
        if let Some(members) = member_data {
            if let Some(member_info) = crate::acbl::lookup_member(
                members,
                &player.number,
                player.name.as_deref(),
            ) {
                sheet.write_string_with_format(row, 8, &member_info.rank, &left_format)?;
                sheet.write_number_with_format(row, 9, member_info.points, &points_format)?;
            }
        }
    }

    Ok(())
}

/// Get sort order for direction (N=0, E=1, S=2, W=3)
fn direction_order(dir: &str) -> i32 {
    match dir {
        "N" => 0,
        "E" => 1,
        "S" => 2,
        "W" => 3,
        _ => 4,
    }
}

/// Write combined PBN (deals) and BWS (scores) data to an Excel file
pub fn write_combined_to_xlsx(
    boards: &[Board],
    bws_data: &crate::bws::BwsData,
    path: &Path,
    member_data: Option<&HashMap<String, crate::acbl::MemberInfo>>,
) -> Result<()> {
    let mut workbook = Workbook::new();

    // Calculate matchpoints once for use in multiple sheets
    let (matchpoints, pair_totals) = calculate_all_matchpoints(bws_data);

    // Add Game Results sheet (with deal info)
    let results_sheet = workbook.add_worksheet();
    write_game_results_with_deals_sheet(results_sheet, bws_data, boards, &matchpoints)?;

    // Add Players sheet with matchpoint totals
    let players_sheet = workbook.add_worksheet();
    write_players_sheet(players_sheet, bws_data, &pair_totals, member_data)?;

    // Add Sections sheet if there are sections
    if !bws_data.sections.is_empty() {
        let sections_sheet = workbook.add_worksheet();
        write_sections_sheet(sections_sheet, bws_data)?;
    }

    // Add Hand Records sheet from PBN
    if !boards.is_empty() {
        let hands_sheet = workbook.add_worksheet();
        write_hand_records_sheet(hands_sheet, boards)?;
    }

    workbook.save(path)?;
    Ok(())
}

/// Write game results with deal information to a worksheet
fn write_game_results_with_deals_sheet(
    sheet: &mut Worksheet,
    data: &crate::bws::BwsData,
    boards: &[Board],
    matchpoints: &[Option<f64>],
) -> Result<()> {
    sheet.set_name("Game Results")?;

    // Build a map of board number to board for quick lookup
    let board_map: HashMap<u32, &Board> = boards
        .iter()
        .filter_map(|b| b.number.map(|n| (n, b)))
        .collect();

    // Calculate scores for all results
    let scores: Vec<Option<i32>> = data.received_data
        .iter()
        .map(|r| calculate_score_for_result(r))
        .collect();

    // Create sorted indices: by Board ascending, then Score descending
    let mut sorted_indices: Vec<usize> = (0..data.received_data.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        let board_cmp = data.received_data[a].board.cmp(&data.received_data[b].board);
        if board_cmp != std::cmp::Ordering::Equal {
            return board_cmp;
        }
        // Score descending (higher scores first)
        let score_a = scores[a].unwrap_or(i32::MIN);
        let score_b = scores[b].unwrap_or(i32::MIN);
        score_b.cmp(&score_a)
    });

    // Set column widths - expanded to include player names
    let col_widths = [
        8,  // Board
        8,  // Section
        6,  // Table
        6,  // Round
        8,  // NS Pair
        8,  // EW Pair
        18, // N Name
        18, // E Name
        18, // S Name
        18, // W Name
        10, // Declarer
        10, // Contract
        8,  // Result
        10, // Lead Card
        8,  // Score
        8,  // NS MP%
        8,  // EW MP%
        6,  // Vul
        16, // North Hand
        16, // East Hand
        16, // South Hand
        16, // West Hand
    ];
    for (col, width) in col_widths.iter().enumerate() {
        sheet.set_column_width(col as u16, *width)?;
    }

    // Header format
    let header_format = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_border_bottom(FormatBorder::Thin);

    // Write headers
    let headers = [
        "Board", "Section", "Table", "Round",
        "NS Pair", "EW Pair", "N Name", "E Name", "S Name", "W Name",
        "Declarer", "Contract", "Result", "Lead",
        "Score", "NS MP%", "EW MP%",
        "Vul", "North", "East", "South", "West",
    ];

    for (col, header) in headers.iter().enumerate() {
        sheet.write_string_with_format(0, col as u16, *header, &header_format)?;
    }

    // Data formats
    let center_format = Format::new().set_align(FormatAlign::Center);
    let score_format = Format::new().set_align(FormatAlign::Right);
    let mp_format = Format::new()
        .set_align(FormatAlign::Right)
        .set_num_format("0.0");
    let left_format = Format::new().set_align(FormatAlign::Left);

    // Write result data in sorted order
    for (row_idx, &original_idx) in sorted_indices.iter().enumerate() {
        let result = &data.received_data[original_idx];
        let row = (row_idx + 1) as u32;

        sheet.write_number_with_format(row, 0, result.board as f64, &center_format)?;
        sheet.write_number_with_format(row, 1, result.section as f64, &center_format)?;
        sheet.write_number_with_format(row, 2, result.table as f64, &center_format)?;
        sheet.write_number_with_format(row, 3, result.round as f64, &center_format)?;
        sheet.write_number_with_format(row, 4, result.pair_ns as f64, &center_format)?;
        sheet.write_number_with_format(row, 5, result.pair_ew as f64, &center_format)?;

        // Player names - look up by pair number (starting table) and direction
        // NS pair started at table = pair_ns, EW pair started at table = pair_ew
        if let Some(n_name) = data.get_player_at(result.section, result.pair_ns, "N") {
            sheet.write_string_with_format(row, 6, n_name, &left_format)?;
        }
        if let Some(e_name) = data.get_player_at(result.section, result.pair_ew, "E") {
            sheet.write_string_with_format(row, 7, e_name, &left_format)?;
        }
        if let Some(s_name) = data.get_player_at(result.section, result.pair_ns, "S") {
            sheet.write_string_with_format(row, 8, s_name, &left_format)?;
        }
        if let Some(w_name) = data.get_player_at(result.section, result.pair_ew, "W") {
            sheet.write_string_with_format(row, 9, w_name, &left_format)?;
        }

        // Declarer direction
        let declarer_dir = match result.ns_ew.as_str() {
            "N" => "North",
            "S" => "South",
            "E" => "East",
            "W" => "West",
            _ => &result.ns_ew,
        };
        sheet.write_string_with_format(row, 10, declarer_dir, &center_format)?;

        sheet.write_string_with_format(row, 11, &result.contract, &center_format)?;
        sheet.write_string_with_format(row, 12, &result.result, &center_format)?;

        if let Some(ref lead) = result.lead_card {
            sheet.write_string_with_format(row, 13, lead, &center_format)?;
        }

        // Score (from NS perspective)
        if let Some(score) = scores[original_idx] {
            sheet.write_number_with_format(row, 14, score as f64, &score_format)?;
        }

        // Matchpoints
        if let Some(mp) = matchpoints[original_idx] {
            sheet.write_number_with_format(row, 15, mp, &mp_format)?;
            sheet.write_number_with_format(row, 16, 100.0 - mp, &mp_format)?;
        }

        // Add deal information if available
        if let Some(board) = board_map.get(&(result.board as u32)) {
            // Vulnerability
            sheet.write_string_with_format(row, 17, board.vulnerable.to_pbn(), &center_format)?;

            // Hands
            for (col_offset, dir) in [
                (18, Direction::North),
                (19, Direction::East),
                (20, Direction::South),
                (21, Direction::West),
            ] {
                let hand = board.deal.hand(dir);
                if hand.len() > 0 {
                    let hand_str = format_hand_compact(hand);
                    sheet.write_string_with_format(row, col_offset, &hand_str, &left_format)?;
                }
            }
        }
    }

    // Add auto-filter to the table
    let last_row = data.received_data.len() as u32;
    let last_col = (headers.len() - 1) as u16;
    sheet.autofilter(0, 0, last_row, last_col)?;

    // Add conditional formatting (3-color scale) to NS MP% and EW MP% columns
    // Red (low) -> Yellow (mid) -> Green (high)
    if !data.received_data.is_empty() {
        let mp_conditional_format = ConditionalFormat3ColorScale::new()
            .set_minimum_color("F8696B") // Red
            .set_midpoint_color("FFEB84") // Yellow
            .set_maximum_color("63BE7B"); // Green

        // NS MP% column (column 15, 0-indexed)
        sheet.add_conditional_format(1, 15, last_row, 15, &mp_conditional_format)?;

        // EW MP% column (column 16, 0-indexed)
        sheet.add_conditional_format(1, 16, last_row, 16, &mp_conditional_format)?;
    }

    Ok(())
}

/// Write sections to a worksheet
fn write_sections_sheet(sheet: &mut Worksheet, data: &crate::bws::BwsData) -> Result<()> {
    sheet.set_name("Sections")?;

    // Set column widths
    sheet.set_column_width(0, 10)?;  // Section
    sheet.set_column_width(1, 8)?;   // Tables
    sheet.set_column_width(2, 12)?;  // Winners
    sheet.set_column_width(3, 14)?;  // Scoring Type

    // Header format
    let header_format = Format::new()
        .set_bold()
        .set_align(FormatAlign::Center)
        .set_border_bottom(FormatBorder::Thin);

    let center_format = Format::new().set_align(FormatAlign::Center);

    // Write headers
    sheet.write_string_with_format(0, 0, "Section", &header_format)?;
    sheet.write_string_with_format(0, 1, "Tables", &header_format)?;
    sheet.write_string_with_format(0, 2, "Winners", &header_format)?;
    sheet.write_string_with_format(0, 3, "Scoring Type", &header_format)?;

    // Write section data
    for (row_idx, section) in data.sections.iter().enumerate() {
        let row = (row_idx + 1) as u32;

        sheet.write_string_with_format(row, 0, section.letter.trim(), &center_format)?;
        sheet.write_number_with_format(row, 1, section.tables as f64, &center_format)?;

        if let Some(winners) = section.winners {
            sheet.write_number_with_format(row, 2, winners as f64, &center_format)?;
        }

        if let Some(scoring) = section.scoring_type {
            let scoring_str = match scoring {
                0 => "Matchpoints",
                1 => "IMPs",
                _ => "Unknown",
            };
            sheet.write_string_with_format(row, 3, scoring_str, &center_format)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_hand_compact() {
        let hand = Hand::from_pbn("AKQ.JT9.876.5432").unwrap();
        let formatted = format_hand_compact(&hand);
        assert!(formatted.contains("SAKQ"));
        assert!(formatted.contains("HJT9"));
    }
}
