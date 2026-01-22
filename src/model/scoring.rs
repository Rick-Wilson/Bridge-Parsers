/// Bridge scoring module for duplicate bridge

/// A parsed contract
#[derive(Debug, Clone, PartialEq)]
pub struct Contract {
    pub level: u8,           // 1-7
    pub strain: Strain,      // Clubs, Diamonds, Hearts, Spades, NoTrump
    pub doubled: Doubled,    // None, Doubled, Redoubled
    pub declarer: char,      // N, E, S, W
}

/// The strain (denomination) of a contract
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Strain {
    Clubs,
    Diamonds,
    Hearts,
    Spades,
    NoTrump,
}

impl Strain {
    /// Whether this is a minor suit (clubs or diamonds)
    pub fn is_minor(&self) -> bool {
        matches!(self, Strain::Clubs | Strain::Diamonds)
    }

    /// Whether this is a major suit (hearts or spades)
    pub fn is_major(&self) -> bool {
        matches!(self, Strain::Hearts | Strain::Spades)
    }

    /// Points per trick for this strain
    pub fn trick_value(&self) -> i32 {
        match self {
            Strain::Clubs | Strain::Diamonds => 20,
            Strain::Hearts | Strain::Spades => 30,
            Strain::NoTrump => 30, // First trick is 40, handled separately
        }
    }

    /// Parse strain from string
    pub fn from_str(s: &str) -> Option<Strain> {
        match s.to_uppercase().as_str() {
            "C" | "CLUBS" => Some(Strain::Clubs),
            "D" | "DIAMONDS" => Some(Strain::Diamonds),
            "H" | "HEARTS" => Some(Strain::Hearts),
            "S" | "SPADES" => Some(Strain::Spades),
            "NT" | "N" | "NOTRUMP" | "NO TRUMP" => Some(Strain::NoTrump),
            _ => None,
        }
    }
}

/// Doubling state
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Doubled {
    #[default]
    None,
    Doubled,
    Redoubled,
}

impl Contract {
    /// Parse a contract string like "3 NT", "4 S X", "6 H XX"
    pub fn parse(s: &str) -> Option<Contract> {
        let s = s.trim().to_uppercase();
        if s.is_empty() || s == "PASS" || s == "PASSED" || s == "AP" || s == "ALL PASS" {
            return None;
        }

        let mut parts: Vec<&str> = s.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        // Parse level (first character or first part)
        let level: u8 = parts[0].chars().next()?.to_digit(10)? as u8;
        if level < 1 || level > 7 {
            return None;
        }

        // Remove level from first part if it's combined (e.g., "3NT" vs "3 NT")
        let first_part = parts[0];
        let strain_start = if first_part.len() > 1 && first_part.chars().next()?.is_ascii_digit() {
            &first_part[1..]
        } else if parts.len() > 1 {
            parts.remove(0);
            parts[0]
        } else {
            return None;
        };

        // Parse strain
        let strain = Strain::from_str(strain_start)?;

        // Check for doubles (X or XX in remaining parts)
        let doubled = if parts.iter().any(|p| *p == "XX" || *p == "REDOUBLED") {
            Doubled::Redoubled
        } else if parts.iter().any(|p| *p == "X" || *p == "DOUBLED") {
            Doubled::Doubled
        } else {
            Doubled::None
        };

        Some(Contract {
            level,
            strain,
            doubled,
            declarer: 'N', // Default, will be set by caller
        })
    }

    /// Parse a result string like "+3", "-1", "="
    pub fn parse_result(s: &str) -> Option<i32> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        if s == "=" || s == "0" || s == "+0" {
            return Some(0);
        }

        // Handle "+N" or "-N" format
        if s.starts_with('+') {
            s[1..].parse::<i32>().ok()
        } else if s.starts_with('-') {
            s.parse::<i32>().ok() // Already negative
        } else {
            s.parse::<i32>().ok()
        }
    }

    /// Calculate the score for this contract
    /// `tricks_relative` is the number of tricks relative to the contract (e.g., +2 for 2 overtricks)
    /// `vulnerable` indicates if the declaring side is vulnerable
    pub fn score(&self, tricks_relative: i32, vulnerable: bool) -> i32 {
        let tricks_made = self.level as i32 + 6 + tricks_relative;

        if tricks_relative < 0 {
            // Contract went down
            self.undertrick_penalty(-tricks_relative, vulnerable)
        } else {
            // Contract made
            self.making_score(tricks_made, vulnerable)
        }
    }

    /// Calculate the score when the contract makes
    fn making_score(&self, tricks_made: i32, vulnerable: bool) -> i32 {
        let contracted_tricks = self.level as i32;
        let overtricks = tricks_made - (contracted_tricks + 6);

        // Base contract value
        let mut contract_value = match self.strain {
            Strain::NoTrump => 40 + (contracted_tricks - 1) * 30,
            _ => contracted_tricks * self.strain.trick_value(),
        };

        // Apply doubling to contract value
        contract_value = match self.doubled {
            Doubled::None => contract_value,
            Doubled::Doubled => contract_value * 2,
            Doubled::Redoubled => contract_value * 4,
        };

        // Check if game or slam bonus applies
        let game_bonus = if contract_value >= 100 {
            if vulnerable { 500 } else { 300 }
        } else {
            50 // Part score bonus
        };

        // Slam bonuses
        let slam_bonus = match self.level {
            6 => if vulnerable { 750 } else { 500 },  // Small slam
            7 => if vulnerable { 1500 } else { 1000 }, // Grand slam
            _ => 0,
        };

        // Overtrick value
        let overtrick_value = match self.doubled {
            Doubled::None => overtricks * self.strain.trick_value(),
            Doubled::Doubled => overtricks * if vulnerable { 200 } else { 100 },
            Doubled::Redoubled => overtricks * if vulnerable { 400 } else { 200 },
        };

        // Insult bonus for making doubled/redoubled
        let insult = match self.doubled {
            Doubled::None => 0,
            Doubled::Doubled => 50,
            Doubled::Redoubled => 100,
        };

        contract_value + game_bonus + slam_bonus + overtrick_value + insult
    }

    /// Calculate undertrick penalty (returned as negative)
    fn undertrick_penalty(&self, undertricks: i32, vulnerable: bool) -> i32 {
        let penalty = match self.doubled {
            Doubled::None => {
                if vulnerable {
                    undertricks * 100
                } else {
                    undertricks * 50
                }
            }
            Doubled::Doubled => {
                if vulnerable {
                    // Vulnerable doubled: 200, 300, 300, 300...
                    match undertricks {
                        1 => 200,
                        n => 200 + (n - 1) * 300,
                    }
                } else {
                    // Not vulnerable doubled: 100, 200, 200, 300, 300...
                    match undertricks {
                        1 => 100,
                        2 => 300,
                        3 => 500,
                        n => 500 + (n - 3) * 300,
                    }
                }
            }
            Doubled::Redoubled => {
                // Redoubled is double the doubled penalty
                let doubled_penalty = if vulnerable {
                    match undertricks {
                        1 => 200,
                        n => 200 + (n - 1) * 300,
                    }
                } else {
                    match undertricks {
                        1 => 100,
                        2 => 300,
                        3 => 500,
                        n => 500 + (n - 3) * 300,
                    }
                };
                doubled_penalty * 2
            }
        };

        -penalty // Return as negative since it's a penalty
    }
}

/// Calculate matchpoints for a set of scores on a board
/// Returns matchpoints for each score (index corresponds to input index)
/// Uses 2 points per comparison win, 1 for tie
pub fn calculate_matchpoints(scores_ns: &[i32]) -> Vec<f64> {
    let n = scores_ns.len();
    if n == 0 {
        return vec![];
    }

    let mut matchpoints = vec![0.0; n];
    let max_mp = (n - 1) as f64 * 2.0;

    for i in 0..n {
        for j in 0..n {
            if i != j {
                if scores_ns[i] > scores_ns[j] {
                    matchpoints[i] += 2.0;
                } else if scores_ns[i] == scores_ns[j] {
                    matchpoints[i] += 1.0;
                }
            }
        }
    }

    // Convert to percentage
    if max_mp > 0.0 {
        for mp in &mut matchpoints {
            *mp = (*mp / max_mp) * 100.0;
        }
    }

    matchpoints
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_contract() {
        let c = Contract::parse("3 NT").unwrap();
        assert_eq!(c.level, 3);
        assert_eq!(c.strain, Strain::NoTrump);
        assert_eq!(c.doubled, Doubled::None);

        let c = Contract::parse("4 S").unwrap();
        assert_eq!(c.level, 4);
        assert_eq!(c.strain, Strain::Spades);

        let c = Contract::parse("2 H X").unwrap();
        assert_eq!(c.level, 2);
        assert_eq!(c.strain, Strain::Hearts);
        assert_eq!(c.doubled, Doubled::Doubled);

        let c = Contract::parse("6 C XX").unwrap();
        assert_eq!(c.level, 6);
        assert_eq!(c.strain, Strain::Clubs);
        assert_eq!(c.doubled, Doubled::Redoubled);
    }

    #[test]
    fn test_parse_result() {
        assert_eq!(Contract::parse_result("+3"), Some(3));
        assert_eq!(Contract::parse_result("-1"), Some(-1));
        assert_eq!(Contract::parse_result("="), Some(0));
        assert_eq!(Contract::parse_result("+0"), Some(0));
    }

    #[test]
    fn test_score_1nt_making_3() {
        // 1NT+3 not vul = 40 + 30 + 30 + 30 + 300 (game) = 430?
        // Actually: 1NT+3 = 40 + 90 (3 overtricks * 30) + 300 = 430
        // Wait, let me recalculate: 1NT making 4 = contract of 1NT (40) + game bonus (300) + overtricks (3*30=90) = 430
        // But 1NT+3 means 10 tricks, which is 1NT contract value of 40, + 3 overtricks at 30 each = 90
        // Total contract points = 40, so no game bonus, just partscore 50
        // So: 40 + 50 + 90 = 180
        let c = Contract::parse("1 NT").unwrap();
        assert_eq!(c.score(3, false), 180);
    }

    #[test]
    fn test_score_3nt_making() {
        // 3NT= not vul = 40 + 60 + 300 (game) = 400
        let c = Contract::parse("3 NT").unwrap();
        assert_eq!(c.score(0, false), 400);

        // 3NT= vul = 40 + 60 + 500 = 600
        assert_eq!(c.score(0, true), 600);
    }

    #[test]
    fn test_score_4s_making() {
        // 4S= not vul = 120 + 300 = 420
        let c = Contract::parse("4 S").unwrap();
        assert_eq!(c.score(0, false), 420);
    }

    #[test]
    fn test_score_down() {
        // 3NT-1 not vul = -50
        let c = Contract::parse("3 NT").unwrap();
        assert_eq!(c.score(-1, false), -50);

        // 3NT-1 vul = -100
        assert_eq!(c.score(-1, true), -100);

        // 3NT-2 doubled not vul = -300
        let c = Contract::parse("3 NT X").unwrap();
        assert_eq!(c.score(-2, false), -300);
    }

    #[test]
    fn test_matchpoints() {
        // Scores: 420, 420, 400, -50
        let scores = vec![420, 420, 400, -50];
        let mps = calculate_matchpoints(&scores);

        // 420 beats 400 and -50 (4 points), ties with 420 (1 point) = 5/6 = 83.33%
        assert!((mps[0] - 83.33).abs() < 0.1);
        assert!((mps[1] - 83.33).abs() < 0.1);
        // 400 beats -50 (2 points), loses to both 420s = 2/6 = 33.33%
        assert!((mps[2] - 33.33).abs() < 0.1);
        // -50 loses to all = 0%
        assert!((mps[3] - 0.0).abs() < 0.1);
    }
}
