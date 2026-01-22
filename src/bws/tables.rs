use serde::Deserialize;

/// A result record from the ReceivedData table
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ReceivedDataRow {
    #[serde(rename = "ID")]
    pub id: i32,
    pub section: i32,
    pub table: i32,
    pub round: i32,
    pub board: i32,
    #[serde(rename = "PairNS")]
    pub pair_ns: i32,
    #[serde(rename = "PairEW")]
    pub pair_ew: i32,
    pub declarer: i32,
    #[serde(rename = "NS/EW")]
    pub ns_ew: String,
    pub contract: String,
    pub result: String,
    pub lead_card: Option<String>,
    pub remarks: Option<String>,
}

/// A player from the PlayerNames table
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayerNameRow {
    #[serde(rename = "ID")]
    pub id: i32,
    pub name: String,
    #[serde(rename = "strID")]
    pub str_id: String,
}

/// A section from the Section table
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct SectionRow {
    #[serde(rename = "ID")]
    pub id: i32,
    pub letter: String,
    pub tables: i32,
    pub missing_pair: i32,
    #[serde(rename = "EWMoveBeforePlay")]
    pub ew_move_before_play: Option<i32>,
    pub session: Option<i32>,
    pub scoring_type: Option<i32>,
    pub winners: Option<i32>,
}

/// A hand record row (if available)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct HandRecordRow {
    pub section: i32,
    pub board: i32,
    pub north_spades: Option<String>,
    pub north_hearts: Option<String>,
    pub north_diamonds: Option<String>,
    pub north_clubs: Option<String>,
    pub east_spades: Option<String>,
    pub east_hearts: Option<String>,
    pub east_diamonds: Option<String>,
    pub east_clubs: Option<String>,
    pub south_spades: Option<String>,
    pub south_hearts: Option<String>,
    pub south_diamonds: Option<String>,
    pub south_clubs: Option<String>,
    pub west_spades: Option<String>,
    pub west_hearts: Option<String>,
    pub west_diamonds: Option<String>,
    pub west_clubs: Option<String>,
}

/// A player number assignment (links section/table/direction to a player)
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PlayerNumberRow {
    pub section: i32,
    pub table: i32,
    pub direction: String,
    pub number: String,
    pub name: Option<String>,
}
