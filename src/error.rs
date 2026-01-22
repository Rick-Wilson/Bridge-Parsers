use thiserror::Error;

#[derive(Error, Debug)]
pub enum BridgeError {
    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Invalid deal notation: {0}")]
    InvalidDeal(String),

    #[error("Invalid direction: {0}")]
    InvalidDirection(String),

    #[error("Invalid suit: {0}")]
    InvalidSuit(String),

    #[error("Invalid rank: {0}")]
    InvalidRank(String),

    #[error("Invalid vulnerability: {0}")]
    InvalidVulnerability(String),

    #[error("BWS error: {0}")]
    Bws(String),

    #[error("mdbtools not found - install with: brew install mdbtools")]
    MdbtoolsNotFound,

    #[error("LIN format error: {0}")]
    Lin(String),

    #[error("URL resolution error: {0}")]
    UrlResolution(String),

    #[error("Rate limited - please wait and retry")]
    RateLimited,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),

    #[error("Excel error: {0}")]
    Excel(#[from] rust_xlsxwriter::XlsxError),
}

pub type Result<T> = std::result::Result<T, BridgeError>;
