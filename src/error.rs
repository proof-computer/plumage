//! Failures from the store, inference, a fetch, or plumage's own record rules.

use crate::fetch::FetchError;
use serde::Serialize;
use std::fmt;

#[derive(Clone, Debug, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

impl From<forthebirds_store::Error> for Error {
    fn from(error: forthebirds_store::Error) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

impl From<FetchError> for Error {
    fn from(error: FetchError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

impl From<forthebirds_inference::Error> for Error {
    fn from(error: forthebirds_inference::Error) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

impl From<forthebirds_net::Error> for Error {
    fn from(error: forthebirds_net::Error) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}
