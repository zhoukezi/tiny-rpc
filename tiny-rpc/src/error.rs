use std::convert::Infallible;

use err_derive::Error;

use crate::rpc::RequestId;

#[derive(Debug, Error)]
pub enum Error {
    #[error(display = "no response for request {}", _0)]
    MissingResponse(RequestId),
    #[error(display = "unrelated respond for request {}", _0)]
    ResponseMismatch(RequestId),
    #[error(display = "server failed to respond for request {}", _0)]
    ServerError(RequestId),
    #[error(display = "io error: {}", _0)]
    Io(std::io::Error),
    #[error(display = "other error: {}", _0)]
    Other(Box<dyn std::error::Error + Sync + Send + 'static>),
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
