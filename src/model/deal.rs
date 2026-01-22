use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    North,
    East,
    South,
    West,
}

impl Direction {
    pub fn from_char(c: char) -> Option<Self> {
        match c.to_ascii_uppercase() {
            'N' => Some(Direction::North),
            'E' => Some(Direction::East),
            'S' => Some(Direction::South),
            'W' => Some(Direction::West),
            _ => None,
        }
    }

    pub fn to_char(&self) -> char {
        match self {
            Direction::North => 'N',
            Direction::East => 'E',
            Direction::South => 'S',
            Direction::West => 'W',
        }
    }

    pub fn next(&self) -> Direction {
        match self {
            Direction::North => Direction::East,
            Direction::East => Direction::South,
            Direction::South => Direction::West,
            Direction::West => Direction::North,
        }
    }

    pub fn partner(&self) -> Direction {
        self.next().next()
    }

    pub fn prev(&self) -> Direction {
        self.next().next().next()
    }

    pub fn all() -> [Direction; 4] {
        [Direction::North, Direction::East, Direction::South, Direction::West]
    }

    /// Returns directions in clockwise order starting from this direction
    pub fn clockwise_from(&self) -> [Direction; 4] {
        [*self, self.next(), self.next().next(), self.next().next().next()]
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Direction::North => write!(f, "North"),
            Direction::East => write!(f, "East"),
            Direction::South => write!(f, "South"),
            Direction::West => write!(f, "West"),
        }
    }
}

use super::hand::Hand;

#[derive(Debug, Clone, Default)]
pub struct Deal {
    pub north: Hand,
    pub east: Hand,
    pub south: Hand,
    pub west: Hand,
}

impl Deal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn hand(&self, direction: Direction) -> &Hand {
        match direction {
            Direction::North => &self.north,
            Direction::East => &self.east,
            Direction::South => &self.south,
            Direction::West => &self.west,
        }
    }

    pub fn hand_mut(&mut self, direction: Direction) -> &mut Hand {
        match direction {
            Direction::North => &mut self.north,
            Direction::East => &mut self.east,
            Direction::South => &mut self.south,
            Direction::West => &mut self.west,
        }
    }

    pub fn set_hand(&mut self, direction: Direction, hand: Hand) {
        match direction {
            Direction::North => self.north = hand,
            Direction::East => self.east = hand,
            Direction::South => self.south = hand,
            Direction::West => self.west = hand,
        }
    }

    /// Format deal in PBN notation: "N:spades.hearts.diamonds.clubs spades.hearts... ..."
    pub fn to_pbn(&self, first: Direction) -> String {
        let mut parts = Vec::with_capacity(4);
        let mut dir = first;
        for _ in 0..4 {
            parts.push(self.hand(dir).to_pbn());
            dir = dir.next();
        }
        format!("{}:{}", first.to_char(), parts.join(" "))
    }

    /// Parse deal from PBN notation
    pub fn from_pbn(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.len() < 3 {
            return None;
        }

        let first_dir = Direction::from_char(s.chars().next()?)?;
        if s.chars().nth(1)? != ':' {
            return None;
        }

        let hands_str = &s[2..];
        let hand_strs: Vec<&str> = hands_str.split_whitespace().collect();
        if hand_strs.len() != 4 {
            return None;
        }

        let mut deal = Deal::new();
        let mut dir = first_dir;
        for hand_str in hand_strs {
            let hand = Hand::from_pbn(hand_str)?;
            deal.set_hand(dir, hand);
            dir = dir.next();
        }

        Some(deal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_next() {
        assert_eq!(Direction::North.next(), Direction::East);
        assert_eq!(Direction::East.next(), Direction::South);
        assert_eq!(Direction::South.next(), Direction::West);
        assert_eq!(Direction::West.next(), Direction::North);
    }

    #[test]
    fn test_direction_partner() {
        assert_eq!(Direction::North.partner(), Direction::South);
        assert_eq!(Direction::East.partner(), Direction::West);
    }

    #[test]
    fn test_direction_from_char() {
        assert_eq!(Direction::from_char('N'), Some(Direction::North));
        assert_eq!(Direction::from_char('e'), Some(Direction::East));
        assert_eq!(Direction::from_char('X'), None);
    }

    #[test]
    fn test_deal_from_pbn() {
        let pbn = "N:K843.T542.J6.863 AQJ7.K.Q75.AT942 962.AJ7.KT82.J75 T5.Q9863.A943.KQ";
        let deal = Deal::from_pbn(pbn).unwrap();

        assert_eq!(deal.north.hcp(), 4);  // K=3 + J=1
        assert_eq!(deal.east.hcp(), 16);  // Spades: AQJ=7, Hearts: K=3, Diamonds: Q=2, Clubs: A=4

        // Round-trip test
        let pbn_out = deal.to_pbn(Direction::North);
        assert_eq!(pbn, pbn_out);
    }
}
