use std::fmt;

#[derive(Debug)]
pub enum Error {
    HttpStatus(u16),
    InvalidTranslation,
    /// An LLM response that could not be parsed/validated as a valid
    /// TranslationResponse. Carries the specific failure reason for logging.
    InvalidResponse(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::HttpStatus(v) => write!(f, "HTTP Status Code: {v}"),
            Error::InvalidTranslation => write!(f, "Invalid Translation"),
            Error::InvalidResponse(reason) => write!(f, "Invalid Response: {reason}"),
        }
    }
}

impl std::error::Error for Error {}
