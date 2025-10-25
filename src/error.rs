use std::fmt;

#[derive(Debug)]
pub enum Error {
    HttpStatus(u16),
    InvalidTranslation,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::HttpStatus(v) => write!(f, "HTTP Status Code: {}", v),
            Error::InvalidTranslation => write!(f, "Invalid Translation"),
        }
    }
}

impl std::error::Error for Error {}
