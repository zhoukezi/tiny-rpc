use std::{
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use futures::{ready, Sink, Stream};
use pin_project::pin_project;

use crate::error::Error;

mod sealed {
    use futures::{Sink, Stream};

    pub trait StreamSealed<T>: Stream {}

    pub trait SinkSealed<T, U>: Sink<U> {}

    impl<T, S: Stream> StreamSealed<T> for S {}

    impl<T, U, S: Sink<U>> SinkSealed<T, U> for S {}
}

pub trait IntoRpcStream<T>: Stream + Sized {
    fn map(item: Self::Item) -> Result<T, Error>;
    fn into_rpc_stream(self) -> RpcStream<T, Self> {
        RpcStream {
            inner: self,
            _marker: PhantomData,
        }
    }
}

impl<T, U, E, S> IntoRpcStream<T> for S
where
    U: Into<T>,
    E: Into<Error>,
    S: Stream<Item = Result<U, E>> + sealed::StreamSealed<T>,
{
    fn map(item: Self::Item) -> Result<T, Error> {
        item.map(Into::into).map_err(Into::into)
    }
}

pub trait IntoRpcSink<T, U>: Sink<U> + Sized {
    fn map(item: T) -> U;
    fn map_err(err: Self::Error) -> Error;
    fn into_rpc_sink(self) -> RpcSink<T, U, Self> {
        RpcSink {
            inner: self,
            _marker: PhantomData,
        }
    }
}

impl<T, U, E, S> IntoRpcSink<T, U> for S
where
    U: From<T>,
    E: Into<Error>,
    S: Sink<U, Error = E> + sealed::SinkSealed<T, U>,
{
    fn map(item: T) -> U {
        item.into()
    }

    fn map_err(err: Self::Error) -> Error {
        err.into()
    }
}

#[pin_project]
pub struct RpcStream<T, S: IntoRpcStream<T>> {
    #[pin]
    inner: S,
    _marker: PhantomData<T>,
}

impl<T, S: IntoRpcStream<T>> Stream for RpcStream<T, S> {
    type Item = Result<T, Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.project();
        let next = ready!(this.inner.poll_next(cx)).map(S::map);
        Poll::Ready(next)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

#[pin_project]
pub struct RpcSink<T, U, S: IntoRpcSink<T, U>> {
    #[pin]
    inner: S,
    _marker: PhantomData<(T, U)>,
}

impl<T, U, S: IntoRpcSink<T, U>> Sink<T> for RpcSink<T, U, S> {
    type Error = Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_ready(cx).map(|res| res.map_err(S::map_err))
    }

    fn start_send(self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let this = self.project();
        this.inner.start_send(S::map(item)).map_err(S::map_err)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_flush(cx).map(|res| res.map_err(S::map_err))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        this.inner.poll_close(cx).map(|res| res.map_err(S::map_err))
    }
}

pub fn into_rpc_stream<T, S: IntoRpcStream<T>>(stream: S) -> RpcStream<T, S> {
    stream.into_rpc_stream()
}

pub fn into_rpc_sink<T, U, S: IntoRpcSink<T, U>>(sink: S) -> RpcSink<T, U, S> {
    sink.into_rpc_sink()
}
