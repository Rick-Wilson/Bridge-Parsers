use super::deal::{Deal, Direction};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Vulnerability {
    #[default]
    None,
    NorthSouth,
    EastWest,
    Both,
}

impl Vulnerability {
    pub fn from_pbn(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "NONE" | "-" | "LOVE" => Some(Vulnerability::None),
            "NS" | "N-S" => Some(Vulnerability::NorthSouth),
            "EW" | "E-W" => Some(Vulnerability::EastWest),
            "BOTH" | "ALL" => Some(Vulnerability::Both),
            _ => None,
        }
    }

    pub fn to_pbn(&self) -> &'static str {
        match self {
            Vulnerability::None => "None",
            Vulnerability::NorthSouth => "NS",
            Vulnerability::EastWest => "EW",
            Vulnerability::Both => "All",
        }
    }

    pub fn is_vulnerable(&self, direction: Direction) -> bool {
        match self {
            Vulnerability::None => false,
            Vulnerability::Both => true,
            Vulnerability::NorthSouth => {
                matches!(direction, Direction::North | Direction::South)
            }
            Vulnerability::EastWest => matches!(direction, Direction::East | Direction::West),
        }
    }

    /// Calculate vulnerability from board number using standard rotation
    pub fn from_board_number(board: u32) -> Self {
        match (board - 1) % 16 {
            0 | 7 | 10 | 13 => Vulnerability::None,
            1 | 4 | 11 | 14 => Vulnerability::NorthSouth,
            2 | 5 | 8 | 15 => Vulnerability::EastWest,
            3 | 6 | 9 | 12 => Vulnerability::Both,
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for Vulnerability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Vulnerability::None => write!(f, "None Vul"),
            Vulnerability::NorthSouth => write!(f, "N-S Vul"),
            Vulnerability::EastWest => write!(f, "E-W Vul"),
            Vulnerability::Both => write!(f, "Both Vul"),
        }
    }
}

/// Calculate dealer from board number using standard rotation
pub fn dealer_from_board_number(board: u32) -> Direction {
    match (board - 1) % 4 {
        0 => Direction::North,
        1 => Direction::East,
        2 => Direction::South,
        3 => Direction::West,
        _ => unreachable!(),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Board {
    // Identification
    pub number: Option<u32>,
    pub event: Option<String>,
    pub site: Option<String>,
    pub date: Option<String>,

    // Setup
    pub dealer: Option<Direction>,
    pub vulnerable: Vulnerability,
    pub deal: Deal,

    // Analysis (from PBN)
    pub double_dummy_tricks: Option<String>,
    pub optimum_score: Option<String>,
    pub par_contract: Option<String>,
}

impl Board {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_number(mut self, number: u32) -> Self {
        self.number = Some(number);
        self
    }

    pub fn with_dealer(mut self, dealer: Direction) -> Self {
        self.dealer = Some(dealer);
        self
    }

    pub fn with_vulnerability(mut self, vuln: Vulnerability) -> Self {
        self.vulnerable = vuln;
        self
    }

    pub fn with_deal(mut self, deal: Deal) -> Self {
        self.deal = deal;
        self
    }

    pub fn title(&self) -> String {
        let mut parts = Vec::new();

        if let Some(num) = self.number {
            parts.push(format!("Board {}", num));
        }

        if let Some(dealer) = self.dealer {
            parts.push(format!("{} Deals", dealer));
        }

        parts.push(self.vulnerable.to_string());

        parts.join(" • ")
    }

    /// Get HCP for a specific direction
    pub fn hcp(&self, direction: Direction) -> u8 {
        self.deal.hand(direction).hcp()
    }

    /// Get all HCP as [N, E, S, W]
    pub fn all_hcp(&self) -> [u8; 4] {
        [
            self.deal.north.hcp(),
            self.deal.east.hcp(),
            self.deal.south.hcp(),
            self.deal.west.hcp(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vulnerability_parsing() {
        assert_eq!(Vulnerability::from_pbn("None"), Some(Vulnerability::None));
        assert_eq!(Vulnerability::from_pbn("-"), Some(Vulnerability::None));
        assert_eq!(
            Vulnerability::from_pbn("NS"),
            Some(Vulnerability::NorthSouth)
        );
        assert_eq!(
            Vulnerability::from_pbn("E-W"),
            Some(Vulnerability::EastWest)
        );
        assert_eq!(Vulnerability::from_pbn("Both"), Some(Vulnerability::Both));
        assert_eq!(Vulnerability::from_pbn("All"), Some(Vulnerability::Both));
    }

    #[test]
    fn test_vulnerability_check() {
        assert!(!Vulnerability::None.is_vulnerable(Direction::North));
        assert!(Vulnerability::Both.is_vulnerable(Direction::North));
        assert!(Vulnerability::NorthSouth.is_vulnerable(Direction::South));
        assert!(!Vulnerability::NorthSouth.is_vulnerable(Direction::East));
    }

    #[test]
    fn test_vulnerability_from_board() {
        // Board 1: None, Board 2: NS, Board 3: EW, Board 4: Both
        assert_eq!(Vulnerability::from_board_number(1), Vulnerability::None);
        assert_eq!(Vulnerability::from_board_number(2), Vulnerability::NorthSouth);
        assert_eq!(Vulnerability::from_board_number(3), Vulnerability::EastWest);
        assert_eq!(Vulnerability::from_board_number(4), Vulnerability::Both);

        // Cycle repeats
        assert_eq!(Vulnerability::from_board_number(17), Vulnerability::None);
    }

    #[test]
    fn test_dealer_from_board() {
        assert_eq!(dealer_from_board_number(1), Direction::North);
        assert_eq!(dealer_from_board_number(2), Direction::East);
        assert_eq!(dealer_from_board_number(3), Direction::South);
        assert_eq!(dealer_from_board_number(4), Direction::West);
        assert_eq!(dealer_from_board_number(5), Direction::North);
    }

    #[test]
    fn test_board_title() {
        let board = Board::new()
            .with_number(1)
            .with_dealer(Direction::North)
            .with_vulnerability(Vulnerability::None);

        assert_eq!(board.title(), "Board 1 • North Deals • None Vul");
    }
}
