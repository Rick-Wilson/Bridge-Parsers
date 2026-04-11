use crate::{Auction, Board, Direction, PlaySequence};

/// Write boards to PBN format
pub fn write_pbn(boards: &[Board]) -> String {
    let mut output = String::new();

    // PBN header
    output.push_str("% PBN 2.1\n");
    output.push_str("% EXPORT\n");
    output.push('\n');

    for (i, board) in boards.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        output.push_str(&board_to_pbn(board));
    }

    output
}

/// Convert a single board to PBN format
pub fn board_to_pbn(board: &Board) -> String {
    let mut lines = Vec::new();

    // Event tag
    if let Some(ref event) = board.event {
        lines.push(format!("[Event \"{}\"]", event));
    } else {
        lines.push("[Event \"\"]".to_string());
    }

    // Site tag
    if let Some(ref site) = board.site {
        lines.push(format!("[Site \"{}\"]", site));
    } else {
        lines.push("[Site \"\"]".to_string());
    }

    // Date tag
    if let Some(ref date) = board.date {
        lines.push(format!("[Date \"{}\"]", date));
    } else {
        lines.push("[Date \"\"]".to_string());
    }

    // Board number
    if let Some(num) = board.number {
        lines.push(format!("[Board \"{}\"]", num));
    }

    // Player names
    if let Some(ref names) = board.player_names {
        lines.push(format!(
            "[West \"{}\"]",
            names.west.as_deref().unwrap_or("")
        ));
        lines.push(format!(
            "[North \"{}\"]",
            names.north.as_deref().unwrap_or("")
        ));
        lines.push(format!(
            "[East \"{}\"]",
            names.east.as_deref().unwrap_or("")
        ));
        lines.push(format!(
            "[South \"{}\"]",
            names.south.as_deref().unwrap_or("")
        ));
    } else {
        lines.push("[West \"\"]".to_string());
        lines.push("[North \"\"]".to_string());
        lines.push("[East \"\"]".to_string());
        lines.push("[South \"\"]".to_string());
    }

    // Dealer
    if let Some(dealer) = board.dealer {
        lines.push(format!("[Dealer \"{}\"]", dealer.to_char()));
    }

    // Vulnerability
    lines.push(format!("[Vulnerable \"{}\"]", board.vulnerable.to_pbn()));

    // Deal
    let first_dir = board.dealer.unwrap_or(Direction::North);
    lines.push(format!("[Deal \"{}\"]", board.deal.to_pbn(first_dir)));

    // Scoring
    lines.push("[Scoring \"\"]".to_string());

    // Declarer
    if let Some(declarer) = board.declarer {
        lines.push(format!("[Declarer \"{}\"]", declarer.to_char()));
    } else {
        lines.push("[Declarer \"\"]".to_string());
    }

    // Contract
    if let Some(ref contract) = board.contract {
        lines.push(format!("[Contract \"{}\"]", contract));
    } else {
        lines.push("[Contract \"\"]".to_string());
    }

    // Result
    if let Some(result) = board.result {
        lines.push(format!("[Result \"{}\"]", result));
    } else {
        lines.push("[Result \"\"]".to_string());
    }

    // Analysis tags if present
    if let Some(ref dd) = board.double_dummy_tricks {
        lines.push(format!("[DoubleDummyTricks \"{}\"]", dd));
    }
    if let Some(ref opt) = board.optimum_score {
        lines.push(format!("[OptimumScore \"{}\"]", opt));
    }
    if let Some(ref par) = board.par_contract {
        lines.push(format!("[ParContract \"{}\"]", par));
    }

    // Auction
    if let Some(ref auction) = board.auction {
        if let Some(dealer) = board.dealer {
            lines.push(format!("[Auction \"{}\"]", dealer.to_char()));
            lines.push(format_auction(auction, dealer));
        }
    }

    // Play
    if let Some(ref play) = board.play {
        lines.push(format!(
            "[Play \"{}\"]",
            play.opening_leader.to_char()
        ));
        lines.push(format_play(play));
    }

    // Commentary
    for comment in &board.commentary {
        lines.push(format!("{{{}}}", comment));
    }

    lines.join("\n") + "\n"
}

/// Format an auction as PBN text (4 calls per line).
fn format_auction(auction: &Auction, _dealer: Direction) -> String {
    let calls = &auction.calls;
    let mut lines = Vec::new();

    for chunk in calls.chunks(4) {
        let row: Vec<String> = chunk
            .iter()
            .map(|ac| {
                let bid_str = ac.call.to_pbn();
                if let Some(ref ann) = ac.annotation {
                    format!("{} ={}=", bid_str, ann)
                } else {
                    bid_str
                }
            })
            .collect();
        lines.push(row.join(" "));
    }

    lines.join("\n")
}

/// Format play sequence as PBN text (one trick per line, 4 cards each).
fn format_play(play: &PlaySequence) -> String {
    let mut lines = Vec::new();

    for trick in &play.tricks {
        let cards: Vec<String> = trick
            .cards
            .iter()
            .map(|opt| {
                opt.map(|c| format!("{}{}", c.suit.to_char(), c.rank.to_char()))
                    .unwrap_or_else(|| "-".to_string())
            })
            .collect();
        lines.push(cards.join(" "));
    }

    lines.join("\n")
}

/// Write boards to a PBN file
pub fn write_pbn_file(boards: &[Board], path: &std::path::Path) -> std::io::Result<()> {
    let content = write_pbn(boards);
    std::fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Deal, Vulnerability};

    #[test]
    fn test_write_simple_board() {
        let deal =
            Deal::from_pbn("N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ")
                .unwrap();
        let board = Board::new()
            .with_number(1)
            .with_dealer(Direction::North)
            .with_vulnerability(Vulnerability::None)
            .with_deal(deal);

        let pbn = board_to_pbn(&board);

        assert!(pbn.contains("[Board \"1\"]"));
        assert!(pbn.contains("[Dealer \"N\"]"));
        assert!(pbn.contains("[Vulnerable \"None\"]"));
        assert!(pbn.contains(
            "[Deal \"N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ\"]"
        ));
    }

    #[test]
    fn test_write_pbn_header() {
        let boards = vec![];
        let pbn = write_pbn(&boards);

        assert!(pbn.starts_with("% PBN 2.1\n"));
        assert!(pbn.contains("% EXPORT"));
    }
}
