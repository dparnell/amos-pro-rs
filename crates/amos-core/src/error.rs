//! Errors raised by the runtime.

use std::fmt;

pub type Result<T> = std::result::Result<T, AmosError>;

#[derive(Clone, Debug, PartialEq)]
pub enum AmosError {
    /// File is not in the expected format.
    BadFormat,
    /// An AMOS runtime error, with its original error number.
    Runtime(u16),
    /// Error with a custom message (Error n / extension messages).
    Message(String),
    /// File system error.
    Io(String),
}

impl fmt::Display for AmosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AmosError::BadFormat => write!(f, "Bad file format"),
            AmosError::Runtime(n) => write!(f, "{}", crate::errors::message(*n)),
            AmosError::Message(m) => write!(f, "{m}"),
            AmosError::Io(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for AmosError {}
