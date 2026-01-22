use crate::error::{BridgeError, Result};
use crate::model::{Board, Deal, Hand, Holding, Vulnerability, dealer_from_board_number};
use super::tables::*;
use std::path::Path;
use std::process::Command;

/// Data extracted from a BWS file
#[derive(Debug, Default)]
pub struct BwsData {
    pub sections: Vec<SectionRow>,
    pub player_names: Vec<PlayerNameRow>,
    pub player_numbers: Vec<PlayerNumberRow>,
    pub received_data: Vec<ReceivedDataRow>,
    pub hand_records: Vec<HandRecordRow>,
    pub boards: Vec<Board>,
}

impl BwsData {
    pub fn has_hand_records(&self) -> bool {
        !self.hand_records.is_empty()
    }

    pub fn has_results(&self) -> bool {
        !self.received_data.is_empty()
    }

    /// Get player name for a given section, table, and direction
    pub fn get_player_at(&self, section: i32, table: i32, direction: &str) -> Option<&str> {
        self.player_numbers
            .iter()
            .find(|p| p.section == section && p.table == table && p.direction == direction)
            .and_then(|p| p.name.as_deref())
    }

    /// Get pair of player names (North-South or East-West) for a table
    pub fn get_pair_names(&self, section: i32, table: i32, is_ns: bool) -> (Option<&str>, Option<&str>) {
        if is_ns {
            (
                self.get_player_at(section, table, "N"),
                self.get_player_at(section, table, "S"),
            )
        } else {
            (
                self.get_player_at(section, table, "E"),
                self.get_player_at(section, table, "W"),
            )
        }
    }
}

/// Check if mdbtools is installed
fn check_mdbtools() -> Result<()> {
    let output = Command::new("which")
        .arg("mdb-export")
        .output()
        .map_err(|_| BridgeError::MdbtoolsNotFound)?;

    if !output.status.success() {
        return Err(BridgeError::MdbtoolsNotFound);
    }
    Ok(())
}

/// List tables in a BWS file
pub fn list_tables(path: &Path) -> Result<Vec<String>> {
    check_mdbtools()?;

    let output = Command::new("mdb-tables")
        .arg(path)
        .output()?;

    if !output.status.success() {
        return Err(BridgeError::Bws(format!(
            "mdb-tables failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    let tables_str = String::from_utf8_lossy(&output.stdout);
    Ok(tables_str.split_whitespace().map(String::from).collect())
}

/// Export a table as CSV
fn export_table(path: &Path, table: &str) -> Result<String> {
    let output = Command::new("mdb-export")
        .arg(path)
        .arg(table)
        .output()?;

    if !output.status.success() {
        return Err(BridgeError::Bws(format!(
            "mdb-export failed for {}: {}",
            table,
            String::from_utf8_lossy(&output.stderr)
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Read and parse a BWS file
pub fn read_bws(path: &Path) -> Result<BwsData> {
    check_mdbtools()?;

    let tables = list_tables(path)?;
    let mut data = BwsData::default();

    // Read Section table
    if tables.contains(&"Section".to_string()) {
        let csv = export_table(path, "Section")?;
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        for result in reader.deserialize() {
            if let Ok(row) = result {
                data.sections.push(row);
            }
        }
    }

    // Read PlayerNames table
    if tables.contains(&"PlayerNames".to_string()) {
        let csv = export_table(path, "PlayerNames")?;
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        for result in reader.deserialize() {
            if let Ok(row) = result {
                data.player_names.push(row);
            }
        }
    }

    // Read ReceivedData table
    if tables.contains(&"ReceivedData".to_string()) {
        let csv = export_table(path, "ReceivedData")?;
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        for result in reader.deserialize() {
            if let Ok(row) = result {
                data.received_data.push(row);
            }
        }
    }

    // Read PlayerNumbers table (links section/table/direction to players)
    if tables.contains(&"PlayerNumbers".to_string()) {
        let csv = export_table(path, "PlayerNumbers")?;
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        for result in reader.deserialize() {
            if let Ok(row) = result {
                data.player_numbers.push(row);
            }
        }
    }

    // Read HandRecord table if available
    if tables.contains(&"HandRecord".to_string()) {
        let csv = export_table(path, "HandRecord")?;
        let mut reader = csv::Reader::from_reader(csv.as_bytes());
        for result in reader.deserialize() {
            if let Ok(row) = result {
                data.hand_records.push(row);
            }
        }
    }

    // Convert hand records to boards if available
    data.boards = hand_records_to_boards(&data.hand_records);

    Ok(data)
}

/// Convert hand record rows to Board models
fn hand_records_to_boards(records: &[HandRecordRow]) -> Vec<Board> {
    let mut boards = Vec::new();

    for record in records {
        // Parse holdings from string format (binary encoded cards)
        let north = parse_hand_from_bws(
            record.north_spades.as_deref(),
            record.north_hearts.as_deref(),
            record.north_diamonds.as_deref(),
            record.north_clubs.as_deref(),
        );
        let east = parse_hand_from_bws(
            record.east_spades.as_deref(),
            record.east_hearts.as_deref(),
            record.east_diamonds.as_deref(),
            record.east_clubs.as_deref(),
        );
        let south = parse_hand_from_bws(
            record.south_spades.as_deref(),
            record.south_hearts.as_deref(),
            record.south_diamonds.as_deref(),
            record.south_clubs.as_deref(),
        );
        let west = parse_hand_from_bws(
            record.west_spades.as_deref(),
            record.west_hearts.as_deref(),
            record.west_diamonds.as_deref(),
            record.west_clubs.as_deref(),
        );

        let deal = Deal { north, east, south, west };
        let board_num = record.board as u32;

        let board = Board::new()
            .with_number(board_num)
            .with_dealer(dealer_from_board_number(board_num))
            .with_vulnerability(Vulnerability::from_board_number(board_num))
            .with_deal(deal);

        boards.push(board);
    }

    // Sort by board number
    boards.sort_by_key(|b| b.number);
    boards
}

/// Parse a hand from BWS holding strings
/// BWS stores holdings as space-separated card values or binary
fn parse_hand_from_bws(
    spades: Option<&str>,
    hearts: Option<&str>,
    diamonds: Option<&str>,
    clubs: Option<&str>,
) -> Hand {
    Hand {
        spades: parse_holding_from_bws(spades),
        hearts: parse_holding_from_bws(hearts),
        diamonds: parse_holding_from_bws(diamonds),
        clubs: parse_holding_from_bws(clubs),
    }
}

/// Parse a holding from BWS format
/// BWS uses binary encoding where each card is represented by a value
fn parse_holding_from_bws(s: Option<&str>) -> Holding {
    use crate::model::Rank;

    let s = match s {
        Some(s) if !s.is_empty() => s,
        _ => return Holding::new(),
    };

    let mut holding = Holding::new();

    // Try parsing as PBN-style string first (AKQJT9876...)
    for c in s.chars() {
        if let Some(rank) = Rank::from_pbn_char(c) {
            holding.add(rank);
        }
    }

    holding
}

/// Get unique board numbers from received data
pub fn get_board_numbers(data: &BwsData) -> Vec<u32> {
    let mut boards: Vec<u32> = data.received_data
        .iter()
        .map(|r| r.board as u32)
        .collect();
    boards.sort();
    boards.dedup();
    boards
}

/// Get a player name by ID
pub fn get_player_name(data: &BwsData, id: i32) -> Option<&str> {
    data.player_names
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.name.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_mdbtools() {
        // This test will pass if mdbtools is installed
        let result = check_mdbtools();
        assert!(result.is_ok(), "mdbtools should be installed");
    }
}
