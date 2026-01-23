use crate::error::Result;
use crate::model::{Board, Deal, Direction, Vulnerability};
use nom::{
    bytes::complete::{take_until, take_while1},
    character::complete::{char, space0},
    sequence::delimited,
    IResult, Parser,
};

/// A parsed PBN tag pair
#[derive(Debug, Clone)]
pub struct TagPair {
    pub name: String,
    pub value: String,
}

/// Parse a tag name (alphanumeric and underscore)
fn tag_name(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_').parse(input)
}

/// Parse a quoted string value
fn quoted_string(input: &str) -> IResult<&str, &str> {
    delimited(char('"'), take_until("\""), char('"')).parse(input)
}

/// Parse a tag pair: [TagName "value"]
fn tag_pair(input: &str) -> IResult<&str, TagPair> {
    let (input, _) = char('[').parse(input)?;
    let (input, _) = space0.parse(input)?;
    let (input, name) = tag_name(input)?;
    let (input, _) = space0.parse(input)?;
    let (input, value) = quoted_string(input)?;
    let (input, _) = space0.parse(input)?;
    let (input, _) = char(']').parse(input)?;

    Ok((
        input,
        TagPair {
            name: name.to_string(),
            value: value.to_string(),
        },
    ))
}

/// Read boards from PBN content
pub fn read_pbn(content: &str) -> Result<Vec<Board>> {
    let mut boards = Vec::new();
    let mut current_board = Board::new();
    let mut has_content = false;
    let mut in_commentary = false;

    for line in content.lines() {
        let line = line.trim();

        // Track multi-line commentary blocks { ... }
        // Commentary can start and end on same line, or span multiple lines
        if in_commentary {
            if line.contains('}') {
                in_commentary = false;
            }
            continue;
        }

        // Check for start of commentary
        if line.starts_with('{') {
            // If closing brace on same line, it's a single-line comment
            if !line.contains('}') {
                in_commentary = true;
            }
            continue;
        }

        // Empty line may signal end of board (but not inside commentary)
        if line.is_empty() {
            if has_content {
                boards.push(current_board);
                current_board = Board::new();
                has_content = false;
            }
            continue;
        }

        // Skip line comments and directives
        if line.starts_with(';') || line.starts_with('%') {
            continue;
        }

        // Parse tag pair
        if line.starts_with('[') {
            if let Ok((_, tag)) = tag_pair(line) {
                has_content = true;
                apply_tag_to_board(&mut current_board, &tag);
            }
            continue;
        }

        // Other data lines (like OptimumResultTable data) - skip for now
    }

    // Don't forget the last board
    if has_content {
        boards.push(current_board);
    }

    Ok(boards)
}

/// Apply a parsed tag to a board
fn apply_tag_to_board(board: &mut Board, tag: &TagPair) {
    match tag.name.as_str() {
        "Board" => {
            if let Ok(num) = tag.value.parse::<u32>() {
                board.number = Some(num);
            }
        }
        "Dealer" => {
            if let Some(c) = tag.value.chars().next() {
                board.dealer = Direction::from_char(c);
            }
        }
        "Vulnerable" => {
            board.vulnerable = Vulnerability::from_pbn(&tag.value).unwrap_or_default();
        }
        "Deal" => {
            if let Some(deal) = Deal::from_pbn(&tag.value) {
                board.deal = deal;
            }
        }
        "Event" => {
            if !tag.value.is_empty() {
                board.event = Some(tag.value.clone());
            }
        }
        "Site" => {
            if !tag.value.is_empty() {
                board.site = Some(tag.value.clone());
            }
        }
        "Date" => {
            if !tag.value.is_empty() {
                board.date = Some(tag.value.clone());
            }
        }
        "DoubleDummyTricks" => {
            board.double_dummy_tricks = Some(tag.value.clone());
        }
        "OptimumScore" => {
            board.optimum_score = Some(tag.value.clone());
        }
        "ParContract" => {
            board.par_contract = Some(tag.value.clone());
        }
        _ => {
            // Ignore other tags for now
        }
    }
}

/// Read boards from a PBN file
pub fn read_pbn_file(path: &std::path::Path) -> Result<Vec<Board>> {
    let content = std::fs::read_to_string(path)?;
    read_pbn(&content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tag_pair() {
        let (_, tag) = tag_pair("[Board \"1\"]").unwrap();
        assert_eq!(tag.name, "Board");
        assert_eq!(tag.value, "1");
    }

    #[test]
    fn test_parse_deal_tag() {
        let (_, tag) =
            tag_pair("[Deal \"N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ\"]")
                .unwrap();
        assert_eq!(tag.name, "Deal");
    }

    #[test]
    fn test_read_simple_pbn() {
        let pbn = r#"
[Board "1"]
[Dealer "N"]
[Vulnerable "None"]
[Deal "N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ"]
"#;
        let boards = read_pbn(pbn).unwrap();
        assert_eq!(boards.len(), 1);
        assert_eq!(boards[0].number, Some(1));
        assert_eq!(boards[0].dealer, Some(Direction::North));
        assert_eq!(boards[0].vulnerable, Vulnerability::None);
        // Verify deal was parsed
        let deal_str = boards[0].deal.to_pbn(Direction::North);
        assert_eq!(
            deal_str,
            "N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ"
        );
    }

    #[test]
    fn test_read_multiple_boards() {
        let pbn = r#"
[Board "1"]
[Dealer "N"]
[Vulnerable "None"]
[Deal "N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ"]

[Board "2"]
[Dealer "E"]
[Vulnerable "NS"]
[Deal "E:Q7.AKT9.JT3.JT96 J653.QJ8.A.AQ732 K92.654.K954.K84 AT84.732.Q8762.5"]
"#;
        let boards = read_pbn(pbn).unwrap();
        assert_eq!(boards.len(), 2);
        assert_eq!(boards[0].number, Some(1));
        assert_eq!(boards[1].number, Some(2));
        assert_eq!(boards[1].dealer, Some(Direction::East));
        assert_eq!(boards[1].vulnerable, Vulnerability::NorthSouth);
    }

    #[test]
    fn test_read_pbn_with_multiline_commentary() {
        let pbn = r#"
[Board "1"]
[Dealer "N"]
[Vulnerable "None"]
[Deal "N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ"]
{This is a multi-line
commentary that spans

several lines with blank lines inside.}

[Board "2"]
[Dealer "E"]
[Vulnerable "NS"]
[Deal "E:Q7.AKT9.JT3.JT96 J653.QJ8.A.AQ732 K92.654.K954.K84 AT84.732.Q8762.5"]
"#;
        let boards = read_pbn(pbn).unwrap();
        // Should find exactly 2 boards, not more due to empty lines in commentary
        assert_eq!(boards.len(), 2, "Found {} boards instead of 2", boards.len());
        assert_eq!(boards[0].number, Some(1));
        assert_eq!(boards[1].number, Some(2));
        // Verify deals are parsed
        assert_eq!(
            boards[0].deal.to_pbn(Direction::North),
            "N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ"
        );
        assert_eq!(
            boards[1].deal.to_pbn(Direction::East),
            "E:Q7.AKT9.JT3.JT96 J653.QJ8.A.AQ732 K92.654.K954.K84 AT84.732.Q8762.5"
        );
    }
}
