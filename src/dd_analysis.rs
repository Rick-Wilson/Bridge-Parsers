//! Double-dummy analysis for bridge cardplay
//!
//! This module provides DD (double-dummy) analysis of bridge cardplay,
//! computing the cost of each card or trick relative to optimal play.

use crate::lin::LinData;
use crate::model::{Card, Direction, Rank, Suit};
use bridge_solver::cards::{card_of, suit_of};
use bridge_solver::{CutoffCache, Hands, PartialTrick, PatternCache, Solver};
use bridge_solver::{CLUB, DIAMOND, EAST, HEART, NOTRUMP, NORTH, SOUTH, SPADE, WEST};
use std::collections::HashMap;

/// A single DD error with attribution
#[derive(Debug, Clone)]
pub struct DdError {
    /// Player name who made the error
    pub player: String,
    /// Trick number (1-based)
    pub trick_num: usize,
    /// Card position in trick (0=lead, 1=2nd, 2=3rd, 3=4th)
    pub card_position: usize,
    /// The card that was played
    pub card: Card,
    /// DD cost (tricks lost by this play)
    pub cost: u8,
}

/// Configuration for DD analysis
#[derive(Debug, Clone)]
pub struct DdAnalysisConfig {
    /// Whether to use mid-trick analysis (per-card DD) or trick-boundary analysis
    pub mid_trick: bool,
    /// Print debug output for DD values
    pub debug: bool,
}

impl Default for DdAnalysisConfig {
    fn default() -> Self {
        Self {
            mid_trick: false,
            debug: false,
        }
    }
}

impl DdAnalysisConfig {
    /// Create config for mid-trick analysis (more detailed)
    pub fn mid_trick() -> Self {
        Self {
            mid_trick: true,
            debug: false,
        }
    }

    /// Create config for trick-boundary analysis (faster)
    pub fn trick_boundary() -> Self {
        Self {
            mid_trick: false,
            debug: false,
        }
    }

    /// Enable debug output
    pub fn with_debug(mut self) -> Self {
        self.debug = true;
        self
    }
}

/// Result of DD analysis for a single board
#[derive(Debug, Clone)]
pub struct DdAnalysisResult {
    /// Board number if available
    pub board_num: Option<usize>,
    /// Contract string (e.g., "3NT", "4SX")
    pub contract: String,
    /// Declarer direction as string
    pub declarer: String,
    /// Initial DD result (tricks declarer can make with optimal play)
    pub initial_dd: u8,
    /// Final result (tricks declarer actually made)
    pub final_result: u8,
    /// All DD errors found
    pub errors: Vec<DdError>,
}

/// Analyze DD errors for a single board
///
/// Returns detailed DD analysis including all errors found during cardplay.
pub fn analyze_board(lin_data: &LinData, config: &DdAnalysisConfig) -> Option<DdAnalysisResult> {
    // Skip passed out hands
    if lin_data.play.is_empty() {
        return None;
    }

    // Extract contract info
    let contract = extract_contract(lin_data);
    if contract == "Passed Out" {
        return None;
    }

    let trump = parse_trump(&contract).ok()?;
    let declarer = extract_declarer(lin_data);
    let declarer_seat = parse_declarer_seat(&declarer).ok()?;
    let initial_leader = (declarer_seat + 1) % 4;
    let declarer_is_ns = declarer_seat == NORTH || declarer_seat == SOUTH;

    // Map seat to player name (pn order is S, W, N, E)
    let seat_to_player: HashMap<usize, String> = [
        (SOUTH, lin_data.player_names[0].clone()),
        (WEST, lin_data.player_names[1].clone()),
        (NORTH, lin_data.player_names[2].clone()),
        (EAST, lin_data.player_names[3].clone()),
    ]
    .into_iter()
    .collect();

    // Convert deal to solver format
    let pbn = lin_data.deal.to_pbn(Direction::North);
    let mut current_hands = Hands::from_pbn(&pbn)?;

    let mut cutoff_cache = CutoffCache::new(16);
    let mut pattern_cache = PatternCache::new(16);

    // Initial DD
    let initial_ns = solve_position(
        &current_hands,
        trump,
        initial_leader,
        &mut cutoff_cache,
        &mut pattern_cache,
    );
    let initial_dd = if declarer_is_ns {
        initial_ns
    } else {
        13 - initial_ns
    };

    // Parse cardplay into tricks
    let cardplay = lin_data.format_cardplay_by_trick();
    let tricks = parse_cardplay(&cardplay).ok()?;

    let mut errors = Vec::new();
    let mut declarer_tricks_won: u8 = 0;
    let mut current_leader = initial_leader;

    if config.mid_trick {
        // Mid-trick mode: compute DD before and after every card
        // Key fix: dd_before for card N should equal dd_after for card N-1
        // This ensures we only count an error when DD actually drops due to THIS card
        for (trick_idx, trick) in tricks.iter().enumerate() {
            let mut seat = current_leader;
            let mut partial_trick = PartialTrick::new();
            let mut cards_in_trick: Vec<(usize, usize)> = Vec::new();

            // Compute DD at start of trick (before any card is played)
            let trick_start_dd = {
                let ns = solve_position(
                    &current_hands,
                    trump,
                    current_leader,
                    &mut cutoff_cache,
                    &mut pattern_cache,
                );
                if declarer_is_ns {
                    declarer_tricks_won + ns
                } else {
                    declarer_tricks_won + (current_hands.num_tricks() as u8).saturating_sub(ns)
                }
            };

            // Track DD as we progress through the trick
            // dd_before for card N = dd_after for card N-1
            let mut current_dd = trick_start_dd;

            for (card_idx, card) in trick.iter().enumerate() {
                let solver_card = bridge_card_to_solver(*card).ok()?;

                // dd_before is the DD state coming into this card
                let dd_before = current_dd;

                // Play the card
                current_hands[seat].remove(solver_card);
                partial_trick.add(solver_card, seat);
                cards_in_trick.push((seat, solver_card));

                // Compute DD AFTER this card is played
                let dd_after = if card_idx == 3 {
                    let winner = determine_trick_winner(&cards_in_trick, trump, current_leader);
                    let declarer_won = if declarer_is_ns {
                        winner == NORTH || winner == SOUTH
                    } else {
                        winner == EAST || winner == WEST
                    };
                    let tricks_from_this = if declarer_won { 1u8 } else { 0u8 };

                    if current_hands.num_tricks() == 0 {
                        declarer_tricks_won + tricks_from_this
                    } else {
                        let ns = solve_position(
                            &current_hands,
                            trump,
                            winner,
                            &mut cutoff_cache,
                            &mut pattern_cache,
                        );
                        if declarer_is_ns {
                            declarer_tricks_won + tricks_from_this + ns
                        } else {
                            let remaining = current_hands.num_tricks() as u8;
                            declarer_tricks_won + tricks_from_this + remaining.saturating_sub(ns)
                        }
                    }
                } else {
                    let (ns, remaining) = solve_mid_trick(
                        &current_hands,
                        trump,
                        &partial_trick,
                        &mut cutoff_cache,
                        &mut pattern_cache,
                    );
                    if declarer_is_ns {
                        declarer_tricks_won + ns
                    } else {
                        declarer_tricks_won + remaining.saturating_sub(ns)
                    }
                };

                // Update current_dd for the next card
                current_dd = dd_after;

                // Debug output
                if config.debug {
                    let card_str = format!(
                        "{}{}",
                        card.suit.letter(),
                        card.rank.to_char()
                    );
                    eprintln!(
                        "  T{} pos{}: {} dd_before={} dd_after={}",
                        trick_idx + 1,
                        card_idx,
                        card_str,
                        dd_before,
                        dd_after
                    );
                }

                // Check for DD change
                let player_is_declarer_side = if declarer_is_ns {
                    seat == NORTH || seat == SOUTH
                } else {
                    seat == EAST || seat == WEST
                };

                let cost = if player_is_declarer_side {
                    if dd_after < dd_before {
                        dd_before - dd_after
                    } else {
                        0
                    }
                } else {
                    // For defenders, DD going up means they made an error
                    if dd_after > dd_before {
                        dd_after - dd_before
                    } else {
                        0
                    }
                };

                if cost > 0 {
                    // Attribute error to correct player
                    // For dummy's cards, attribute to declarer
                    let error_seat = if player_is_declarer_side {
                        declarer_seat // Declarer controls both hands
                    } else {
                        seat
                    };

                    if let Some(player) = seat_to_player.get(&error_seat) {
                        errors.push(DdError {
                            player: player.clone(),
                            trick_num: trick_idx + 1,
                            card_position: card_idx,
                            card: *card,
                            cost,
                        });
                    }
                }

                seat = (seat + 1) % 4;
            }

            // Update state after trick
            if cards_in_trick.len() == 4 {
                let winner = determine_trick_winner(&cards_in_trick, trump, current_leader);
                let declarer_won = if declarer_is_ns {
                    winner == NORTH || winner == SOUTH
                } else {
                    winner == EAST || winner == WEST
                };
                if declarer_won {
                    declarer_tricks_won += 1;
                }
                current_leader = winner;
            }
        }
    } else {
        // Trick-boundary mode: compute DD only at start and end of each trick
        for (trick_idx, trick) in tricks.iter().enumerate() {
            if trick.len() != 4 {
                continue; // Skip incomplete tricks
            }

            let mut seat = current_leader;
            let mut cards_in_trick: Vec<(usize, usize)> = Vec::new();

            // DD at start of trick
            let dd_start = {
                let ns = solve_position(
                    &current_hands,
                    trump,
                    current_leader,
                    &mut cutoff_cache,
                    &mut pattern_cache,
                );
                if declarer_is_ns {
                    declarer_tricks_won + ns
                } else {
                    declarer_tricks_won + (current_hands.num_tricks() as u8).saturating_sub(ns)
                }
            };

            // Track which cards were played in this trick
            let mut trick_cards: Vec<(usize, Card)> = Vec::new();

            // Play all cards in trick
            for (card_idx, card) in trick.iter().enumerate() {
                let solver_card = match bridge_card_to_solver(*card) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                current_hands[seat].remove(solver_card);
                cards_in_trick.push((seat, solver_card));
                trick_cards.push((card_idx, *card));
                seat = (seat + 1) % 4;
            }

            // DD at end of trick
            let winner = determine_trick_winner(&cards_in_trick, trump, current_leader);
            let declarer_won = if declarer_is_ns {
                winner == NORTH || winner == SOUTH
            } else {
                winner == EAST || winner == WEST
            };
            let tricks_from_this = if declarer_won { 1u8 } else { 0u8 };

            let dd_end = if current_hands.num_tricks() == 0 {
                declarer_tricks_won + tricks_from_this
            } else {
                let ns = solve_position(
                    &current_hands,
                    trump,
                    winner,
                    &mut cutoff_cache,
                    &mut pattern_cache,
                );
                if declarer_is_ns {
                    declarer_tricks_won + tricks_from_this + ns
                } else {
                    let remaining = current_hands.num_tricks() as u8;
                    declarer_tricks_won + tricks_from_this + remaining.saturating_sub(ns)
                }
            };

            // Debug output for trick-boundary mode
            if config.debug {
                eprintln!(
                    "  T{}: dd_start={} dd_end={} diff={}",
                    trick_idx + 1,
                    dd_start,
                    dd_end,
                    dd_end as i16 - dd_start as i16
                );
            }

            // Check for DD change
            if dd_end < dd_start {
                // DD dropped for declarer - attribute to declarer
                let cost = dd_start - dd_end;
                if let Some(player) = seat_to_player.get(&declarer_seat) {
                    // For trick-boundary, we don't know exactly which card caused it
                    // Use the first card position (lead) as a marker
                    errors.push(DdError {
                        player: player.clone(),
                        trick_num: trick_idx + 1,
                        card_position: 0, // Unknown within trick
                        card: trick[0],
                        cost,
                    });
                }
            } else if dd_end > dd_start {
                // DD rose for declarer - defense error
                let cost = dd_end - dd_start;

                // Attribute to the leader if they're a defender
                let leader_is_defender = if declarer_is_ns {
                    current_leader == EAST || current_leader == WEST
                } else {
                    current_leader == NORTH || current_leader == SOUTH
                };

                let error_seat = if leader_is_defender {
                    current_leader
                } else {
                    // Find first defender who played
                    cards_in_trick
                        .iter()
                        .map(|(s, _)| *s)
                        .find(|s| {
                            if declarer_is_ns {
                                *s == EAST || *s == WEST
                            } else {
                                *s == NORTH || *s == SOUTH
                            }
                        })
                        .unwrap_or(current_leader)
                };

                if let Some(player) = seat_to_player.get(&error_seat) {
                    errors.push(DdError {
                        player: player.clone(),
                        trick_num: trick_idx + 1,
                        card_position: 0,
                        card: trick[0],
                        cost,
                    });
                }
            }

            // Update state
            if declarer_won {
                declarer_tricks_won += 1;
            }
            current_leader = winner;
        }
    }

    let board_num = extract_board_number(&lin_data.board_header);

    Some(DdAnalysisResult {
        board_num,
        contract,
        declarer,
        initial_dd,
        final_result: declarer_tricks_won,
        errors,
    })
}

/// Aggregate DD errors by player, counting number of errors (not summing trick costs)
///
/// Returns a map of player name -> total error count
pub fn aggregate_errors_by_player(result: &DdAnalysisResult) -> HashMap<String, u8> {
    let mut counts: HashMap<String, u8> = HashMap::new();
    for error in &result.errors {
        *counts.entry(error.player.clone()).or_insert(0) += 1;
    }
    counts
}

/// Aggregate DD errors by player, summing trick costs
///
/// Returns a map of player name -> total tricks lost
pub fn aggregate_costs_by_player(result: &DdAnalysisResult) -> HashMap<String, u8> {
    let mut costs: HashMap<String, u8> = HashMap::new();
    for error in &result.errors {
        *costs.entry(error.player.clone()).or_insert(0) += error.cost;
    }
    costs
}

// Helper functions

fn solve_position(
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

/// Solve mid-trick position and return (NS tricks, total tricks remaining)
///
/// The total tricks remaining is the max hand size, which is what the solver uses internally.
/// This is important for mid-trick positions where hands have different sizes.
fn solve_mid_trick(
    hands: &Hands,
    trump: usize,
    partial_trick: &PartialTrick,
    cutoff_cache: &mut CutoffCache,
    pattern_cache: &mut PatternCache,
) -> (u8, u8) {
    // Max hand size = hands that haven't played yet = total tricks remaining
    let max_hand_size = (0..4).map(|s| hands[s].size()).max().unwrap_or(0) as u8;

    if max_hand_size == 0 {
        return (0, 0);
    }
    if let Some(solver) = Solver::new_mid_trick(*hands, trump, partial_trick) {
        let ns = solver.solve_mid_trick(cutoff_cache, pattern_cache, partial_trick);
        (ns, max_hand_size)
    } else if let Some(leader) = partial_trick.leader() {
        let ns = solve_position(hands, trump, leader, cutoff_cache, pattern_cache);
        (ns, max_hand_size)
    } else {
        (0, max_hand_size)
    }
}

fn extract_board_number(header: &Option<String>) -> Option<usize> {
    header.as_ref().and_then(|h| {
        h.split_whitespace()
            .last()
            .and_then(|n| n.parse().ok())
    })
}

fn extract_contract(lin_data: &LinData) -> String {
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
        return "Passed Out".to_string();
    }

    let mut contract = format!("{}{}", level, suit);
    if redoubled {
        contract.push_str("XX");
    } else if doubled {
        contract.push_str("X");
    }
    contract
}

fn extract_declarer(lin_data: &LinData) -> String {
    if !lin_data.play.is_empty() {
        let opening_lead = &lin_data.play[0];
        for dir in Direction::all() {
            let hand = lin_data.deal.hand(dir);
            if hand.holding(opening_lead.suit).contains(opening_lead.rank) {
                return match dir {
                    Direction::North => "West".to_string(),
                    Direction::East => "North".to_string(),
                    Direction::South => "East".to_string(),
                    Direction::West => "South".to_string(),
                };
            }
        }
    }
    "Unknown".to_string()
}

fn parse_trump(contract: &str) -> Result<usize, String> {
    let contract = contract.trim().to_uppercase();
    if contract.contains("NT") || (contract.contains('N') && !contract.contains('S')) {
        return Ok(NOTRUMP);
    }
    for c in contract.chars() {
        match c {
            'S' => return Ok(SPADE),
            'H' => return Ok(HEART),
            'D' => return Ok(DIAMOND),
            'C' => return Ok(CLUB),
            _ => continue,
        }
    }
    Err(format!("Could not parse trump from: {}", contract))
}

fn parse_declarer_seat(declarer: &str) -> Result<usize, String> {
    match declarer.trim().to_uppercase().chars().next() {
        Some('N') => Ok(NORTH),
        Some('E') => Ok(EAST),
        Some('S') => Ok(SOUTH),
        Some('W') => Ok(WEST),
        _ => Err(format!("Invalid declarer: {}", declarer)),
    }
}

fn parse_cardplay(cardplay: &str) -> Result<Vec<Vec<Card>>, String> {
    let mut tricks = Vec::new();
    for trick_str in cardplay.split('|') {
        if trick_str.is_empty() {
            continue;
        }
        let mut trick = Vec::new();
        // Cards within a trick are separated by spaces (from format_cardplay_by_trick)
        for card_str in trick_str.split_whitespace() {
            let card = parse_card_str(card_str)?;
            trick.push(card);
        }
        if !trick.is_empty() {
            tricks.push(trick);
        }
    }
    Ok(tricks)
}

fn parse_card_str(s: &str) -> Result<Card, String> {
    let s = s.trim();
    if s.len() < 2 {
        return Err(format!("Invalid card: {}", s));
    }
    let mut chars = s.chars();
    let suit_char = chars.next().unwrap();
    let rank_char = chars.next().unwrap();

    let suit = match suit_char.to_ascii_uppercase() {
        'S' => Suit::Spades,
        'H' => Suit::Hearts,
        'D' => Suit::Diamonds,
        'C' => Suit::Clubs,
        _ => return Err(format!("Invalid suit: {}", suit_char)),
    };

    let rank =
        Rank::from_pbn_char(rank_char).ok_or_else(|| format!("Invalid rank: {}", rank_char))?;

    Ok(Card::new(suit, rank))
}

fn bridge_card_to_solver(card: Card) -> Result<usize, String> {
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

fn determine_trick_winner(cards: &[(usize, usize)], trump: usize, leader: usize) -> usize {
    let mut winner_idx = 0;
    let mut winning_card = cards[0].1;

    for (i, (_seat, card)) in cards.iter().enumerate().skip(1) {
        let card_suit = suit_of(*card);
        let beats = if card_suit == suit_of(winning_card) {
            *card < winning_card
        } else if card_suit == trump && trump < NOTRUMP {
            suit_of(winning_card) != trump
        } else {
            false
        };

        if beats {
            winner_idx = i;
            winning_card = *card;
        }
    }

    (leader + winner_idx) % 4
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lin::parse_lin;

    #[test]
    fn test_extract_contract() {
        let lin = "pn|South,West,North,East|md|3SAKHJD876C5432,S2HQT9DKQ5CKQJT9,SQJT9HA32DAJ2CA8,|sv|o|ah|Board+1|mb|1C|mb|p|mb|1N|mb|p|mb|p|mb|p|";
        let data = parse_lin(lin).unwrap();

        let contract = extract_contract(&data);
        assert_eq!(contract, "1N");
    }

    #[test]
    fn test_extract_declarer() {
        // LIN has opening lead, so we can determine declarer
        let lin = "pn|South,West,North,East|md|3SAKHJD876C5432,S2HQT9DKQ5CKQJT9,SQJT9HA32DAJ2CA8,|sv|o|ah|Board+1|mb|1C|mb|p|mb|1N|mb|p|mb|p|mb|p|pc|D2|";
        let data = parse_lin(lin).unwrap();

        let declarer = extract_declarer(&data);
        // Just verify we get a valid direction, not "Unknown"
        assert!(
            declarer == "North" || declarer == "South" || declarer == "East" || declarer == "West",
            "Expected a valid direction, got: {}",
            declarer
        );
    }

    #[test]
    fn test_parse_trump() {
        assert_eq!(parse_trump("1N").unwrap(), NOTRUMP);
        assert_eq!(parse_trump("3NT").unwrap(), NOTRUMP);
        assert_eq!(parse_trump("4S").unwrap(), SPADE);
        assert_eq!(parse_trump("2H").unwrap(), HEART);
        assert_eq!(parse_trump("5D").unwrap(), DIAMOND);
        assert_eq!(parse_trump("3C").unwrap(), CLUB);
    }

    #[test]
    fn test_config() {
        let default = DdAnalysisConfig::default();
        assert!(!default.mid_trick);

        let mid = DdAnalysisConfig::mid_trick();
        assert!(mid.mid_trick);

        let boundary = DdAnalysisConfig::trick_boundary();
        assert!(!boundary.mid_trick);
    }
}
