use thiserror::Error;

#[derive(Debug, Error)]
pub enum GhostError {
    #[error("Data error: {0}")]
    Data(#[from] DataError),
    #[error("Analysis error: {0}")]
    Analysis(#[from] AnalysisError),
    #[error("Alert error: {0}")]
    Alert(#[from] AlertError),
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),
}

#[derive(Debug, Error, PartialEq)]
pub enum DataError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("Rate limit exceeded for {api}")]
    RateLimit { api: String },
    #[error("Ticker not found: {ticker}")]
    NotFound { ticker: String },
    #[error("Insufficient history: need {need} bars, got {got}")]
    InsufficientHistory { need: usize, got: usize },
    #[error("Deserialization failed: {0}")]
    Parse(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum AnalysisError {
    #[error("Not enough candles: need {need}, got {got}")]
    InsufficientData { need: usize, got: usize },
    #[error("ONNX inference failed: {0}")]
    OnnxInference(String),
    #[error("Tokenizer error: {0}")]
    Tokenizer(String),
    #[error("Division by zero in indicator calculation")]
    DivisionByZero,
}

#[derive(Debug, Error, PartialEq)]
pub enum AlertError {
    #[error("Telegram API error: {status} — {body}")]
    TelegramApi { status: u16, body: String },
    #[error("Channel unavailable: {0}")]
    ChannelUnavailable(String),
    #[error("Rate limit: max alerts per day reached")]
    DailyLimitReached,
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Migration failed at version {version}: {reason}")]
    Migration { version: u32, reason: String },
    #[error("Record not found: {0}")]
    NotFound(String),
    #[error("Database lock poisoned — a prior operation panicked")]
    Poisoned,
}

#[derive(Debug, Error)]
pub enum PortfolioError {
    #[error("CSV import parse error at line {line}: {reason}")]
    ImportParse { line: usize, reason: String },
    #[error("Unknown CSV format: could not detect header layout")]
    UnknownFormat,
    #[error("Unknown acquisition type: {0}")]
    UnknownAcquisitionType(String),
    #[error("Invalid lot: {0}")]
    InvalidLot(String),
    #[error("Insufficient shares in lot {lot_id}: need {need}, have {have}")]
    InsufficientShares {
        lot_id: String,
        need: f64,
        have: f64,
    },
}

#[derive(Debug, Error, PartialEq)]
pub enum TaxError {
    #[error("Unknown state: {0}")]
    UnknownState(String),
    #[error("Negative shares in lot {lot_id}")]
    NegativeShares { lot_id: String },
    #[error("Calculation overflow")]
    Overflow,
}

#[derive(Debug, Error, PartialEq)]
pub enum ValuationError {
    #[error("Invalid WACC: {0}")]
    InvalidWacc(String),
    #[error("No convergence in reverse DCF after {iterations} iterations")]
    NoConvergence { iterations: u32 },
    #[error("Missing input: {0}")]
    MissingInput(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum OptimizeError {
    #[error("Solver failed: {0}")]
    SolverFailed(String),
    #[error("Singular covariance matrix")]
    SingularMatrix,
    #[error("Infeasible constraints: {0}")]
    InfeasibleConstraints(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("Config file not found at {path}")]
    FileNotFound { path: String },
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Missing required field: {field}")]
    MissingField { field: String },
    #[error("Invalid value for {field}: {reason}")]
    InvalidValue { field: String, reason: String },
}
