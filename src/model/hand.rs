use super::card::{Rank, Suit};
use std::collections::BTreeSet;
use std::fmt;

/// A holding represents the cards in one suit
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Holding {
    pub ranks: BTreeSet<Rank>,
}

impl Holding {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_ranks(ranks: impl IntoIterator<Item = Rank>) -> Self {
        Self {
            ranks: ranks.into_iter().collect(),
        }
    }

    pub fn add(&mut self, rank: Rank) {
        self.ranks.insert(rank);
    }

    pub fn len(&self) -> usize {
        self.ranks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranks.is_empty()
    }

    pub fn hcp(&self) -> u8 {
        self.ranks.iter().map(|r| r.hcp_value()).sum()
    }

    pub fn contains(&self, rank: Rank) -> bool {
        self.ranks.contains(&rank)
    }

    /// Parse holding from PBN notation (e.g., "AKQ", "T542", "-" for void)
    pub fn from_pbn(s: &str) -> Option<Self> {
        if s == "-" || s.is_empty() {
            return Some(Self::new());
        }

        let mut holding = Self::new();
        for c in s.chars() {
            let rank = Rank::from_pbn_char(c)?;
            holding.add(rank);
        }
        Some(holding)
    }

    /// Format holding in PBN notation
    pub fn to_pbn(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        // Ranks are sorted by BTreeSet, but in reverse order of what we want
        // We want Ace first (highest), so collect and reverse
        let mut ranks: Vec<_> = self.ranks.iter().collect();
        ranks.sort(); // Already sorted by Ord which puts Ace first
        ranks.iter().map(|r| r.to_char()).collect()
    }
}

impl fmt::Display for Holding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            write!(f, "-")
        } else {
            write!(f, "{}", self.to_pbn())
        }
    }
}

/// A hand represents all 13 cards held by one player
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hand {
    pub spades: Holding,
    pub hearts: Holding,
    pub diamonds: Holding,
    pub clubs: Holding,
}

impl Hand {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn holding(&self, suit: Suit) -> &Holding {
        match suit {
            Suit::Spades => &self.spades,
            Suit::Hearts => &self.hearts,
            Suit::Diamonds => &self.diamonds,
            Suit::Clubs => &self.clubs,
        }
    }

    pub fn holding_mut(&mut self, suit: Suit) -> &mut Holding {
        match suit {
            Suit::Spades => &mut self.spades,
            Suit::Hearts => &mut self.hearts,
            Suit::Diamonds => &mut self.diamonds,
            Suit::Clubs => &mut self.clubs,
        }
    }

    pub fn hcp(&self) -> u8 {
        self.spades.hcp() + self.hearts.hcp() + self.diamonds.hcp() + self.clubs.hcp()
    }

    pub fn len(&self) -> usize {
        self.spades.len() + self.hearts.len() + self.diamonds.len() + self.clubs.len()
    }

    /// Returns the shape as [spades, hearts, diamonds, clubs]
    pub fn shape(&self) -> [usize; 4] {
        [
            self.spades.len(),
            self.hearts.len(),
            self.diamonds.len(),
            self.clubs.len(),
        ]
    }

    /// Returns the shape sorted descending (e.g., [5, 4, 3, 1])
    pub fn shape_pattern(&self) -> [usize; 4] {
        let mut shape = self.shape();
        shape.sort_by(|a, b| b.cmp(a));
        shape
    }

    /// Parse hand from PBN notation (e.g., "AKQ.JT9.876.5432")
    pub fn from_pbn(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() != 4 {
            return None;
        }

        Some(Hand {
            spades: Holding::from_pbn(parts[0])?,
            hearts: Holding::from_pbn(parts[1])?,
            diamonds: Holding::from_pbn(parts[2])?,
            clubs: Holding::from_pbn(parts[3])?,
        })
    }

    /// Format hand in PBN notation
    pub fn to_pbn(&self) -> String {
        format!(
            "{}.{}.{}.{}",
            self.spades.to_pbn(),
            self.hearts.to_pbn(),
            self.diamonds.to_pbn(),
            self.clubs.to_pbn()
        )
    }
}

impl fmt::Display for Hand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "♠{} ♥{} ♦{} ♣{}",
            self.spades, self.hearts, self.diamonds, self.clubs
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_holding_from_pbn() {
        let h = Holding::from_pbn("AKQ").unwrap();
        assert_eq!(h.len(), 3);
        assert!(h.contains(Rank::Ace));
        assert!(h.contains(Rank::King));
        assert!(h.contains(Rank::Queen));
        assert_eq!(h.hcp(), 9);
    }

    #[test]
    fn test_holding_void() {
        let h = Holding::from_pbn("-").unwrap();
        assert!(h.is_empty());
        assert_eq!(h.to_pbn(), "");
    }

    #[test]
    fn test_hand_from_pbn() {
        let hand = Hand::from_pbn("AKQ.JT9.876.5432").unwrap();
        assert_eq!(hand.spades.len(), 3);
        assert_eq!(hand.hearts.len(), 3);
        assert_eq!(hand.diamonds.len(), 3);
        assert_eq!(hand.clubs.len(), 4);
        assert_eq!(hand.len(), 13);
        assert_eq!(hand.hcp(), 10); // AKQ + JT9
    }

    #[test]
    fn test_hand_with_void() {
        let hand = Hand::from_pbn("AKQJT98765432...").unwrap();
        assert_eq!(hand.spades.len(), 13);
        assert!(hand.hearts.is_empty());
        assert!(hand.diamonds.is_empty());
        assert!(hand.clubs.is_empty());
    }

    #[test]
    fn test_hand_shape() {
        let hand = Hand::from_pbn("AKQ.JT98.76.5432").unwrap();
        assert_eq!(hand.shape(), [3, 4, 2, 4]);
        assert_eq!(hand.shape_pattern(), [4, 4, 3, 2]);
    }

    #[test]
    fn test_hand_round_trip() {
        let original = "K843.T542.J6.863";
        let hand = Hand::from_pbn(original).unwrap();
        assert_eq!(hand.to_pbn(), original);
    }
}
