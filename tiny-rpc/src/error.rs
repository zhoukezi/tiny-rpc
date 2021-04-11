use std::{convert::Infallible, error::Error as StdError, fmt::Display};

use crate::io::Id;

#[derive(Debug)]
pub enum Error {
    Protocol(ProtocolError),
    Io(std::io::Error),
    Serialize(Option<bincode::Error>),
    Driver,
    Spawner,
    Other(Box<dyn StdError + Send + Sync + 'static>),
    Unspecified,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Protocol(e) => {
                write!(f, "protocol error: {}", e)
            }
            Error::Io(e) => {
                write!(f, "io error: {}", e)
            }
            Error::Serialize(e) => {
                write!(f, "serialize error: ")?;
                if let Some(e) = e {
                    write!(f, "{}", e)
                } else {
                    write!(f, "internal error")
                }
            }
            Error::Driver => {
                write!(f, "driver stopped unexpectedly")
            }
            Error::Spawner => {
                write!(f, "spawner stopped unexpectedly")
            }
            Error::Other(e) => {
                write!(f, "other error: {}", e)
            }
            Error::Unspecified => {
                write!(f, "unspecified error")
            }
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Serialize(e) => e.as_ref().map(|e| e.as_ref() as &(dyn StdError + 'static)),
            Error::Other(e) => Some(e.as_ref() as &(dyn StdError + 'static)),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<Infallible> for Error {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}

impl<E: std::error::Error + Sync + Send + 'static> From<Box<E>> for Error {
    fn from(e: Box<E>) -> Self {
        Self::Other(e)
    }
}

// TODO more details
#[derive(Debug)]
pub enum ProtocolError {
    MartianResponse,
    ResponseMismatch(Id),
}

impl Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::MartianResponse => {
                write!(f, "response of non-exist request")
            }
            ProtocolError::ResponseMismatch(_) => {
                write!(f, "response for different function")
            }
        }
    }
}

impl StdError for ProtocolError {}

impl From<ProtocolError> for Error {
    fn from(e: ProtocolError) -> Self {
        Self::Protocol(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
