use std::{
    convert::TryInto,
    mem::size_of,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use bincode::{deserialize_from, serialize_into, serialized_size};
use bytes::{BufMut, Bytes, BytesMut};
use futures::{channel::mpsc, future::ready, Sink, SinkExt, Stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use tokio::io::{split, AsyncRead, AsyncWrite};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

use crate::error::{Error, Result};

#[repr(transparent)]
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Id(u64);

impl Id {
    pub const NULL: Id = Id(0);
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{:016X}]", self.0)
    }
}

#[derive(Clone)]
pub struct IdGenerator(Arc<AtomicU64>);

impl IdGenerator {
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(5)))
    }

    pub fn next(&self) -> Id {
        Id(self.0.fetch_add(1, Ordering::SeqCst))
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

pub struct RpcFrame(Bytes);

impl RpcFrame {
    pub fn new<T: Serialize>(id: Id, data: T) -> Result<Self> {
        let cap = size_of::<Id>() + serialized_size(&data)? as usize;
        let mut buf = BytesMut::with_capacity(cap);
        buf.put_u64(id.0);
        let mut writer = buf.writer();
        serialize_into(&mut writer, &data)?;
        let buf = writer.into_inner();
        assert_eq!(cap, buf.capacity());
        Ok(Self(buf.freeze()))
    }

    pub fn id(&self) -> Result<Id> {
        self.0
            .get(0..size_of::<Id>())
            .map(|buf| {
                Id(u64::from_be_bytes(
                    buf.try_into().expect("infallible: hardcode slice size"),
                ))
            })
            .ok_or(Error::Serialize(None))
    }

    pub fn data<T: DeserializeOwned>(&self) -> Result<T> {
        Ok(deserialize_from(
            self.0
                .get(size_of::<Id>()..)
                .ok_or(Error::Serialize(None))?,
        )?)
    }
}

pub type GenericStream<T> = Pin<Box<dyn Stream<Item = T> + Send + Sync + 'static>>;
pub type GenericSink<T, E> = Pin<Box<dyn Sink<T, Error = E> + Send + Sync + 'static>>;

pub struct Transport {
    input: GenericStream<Result<RpcFrame>>,
    output: GenericSink<RpcFrame, Error>,
}

impl Transport {
    pub fn from_streamed<T>(io: T) -> Self
    where
        T: AsyncRead + AsyncWrite + Send + Sync + 'static,
    {
        let (reader, writer) = split(io);
        Self::from_streamed_pair(reader, writer)
    }

    pub fn from_streamed_pair<R, W>(reader: R, writer: W) -> Self
    where
        R: AsyncRead + Send + Sync + 'static,
        W: AsyncWrite + Send + Sync + 'static,
    {
        let stream = FramedRead::new(reader, LengthDelimitedCodec::default())
            .map(|buf| buf.map(BytesMut::freeze).map(RpcFrame).map_err(Error::from));
        let sink = FramedWrite::new(writer, LengthDelimitedCodec::default())
            .with(|frame: RpcFrame| ready(Ok(frame.0)));
        Self::from_framed_pair(stream, sink)
    }

    pub fn from_framed<T>(io: T) -> Self
    where
        T: Stream<Item = Result<RpcFrame>> + Sink<RpcFrame, Error = Error> + Send + Sync + 'static,
    {
        let (sink, stream) = io.split();
        Self::from_framed_pair(stream, sink)
    }

    pub fn from_framed_pair<T, U>(stream: T, sink: U) -> Self
    where
        T: Stream<Item = Result<RpcFrame>> + Send + Sync + 'static,
        U: Sink<RpcFrame, Error = Error> + Send + Sync + 'static,
    {
        Self {
            input: Box::pin(stream),
            output: Box::pin(sink),
        }
    }

    pub fn new_local() -> (Self, Self) {
        let (tx1, rx1) = mpsc::unbounded::<RpcFrame>();
        let (tx2, rx2) = mpsc::unbounded::<RpcFrame>();

        let tx1 = tx1.sink_map_err(|_| Error::Io(std::io::ErrorKind::ConnectionAborted.into()));
        let tx2 = tx2.sink_map_err(|_| Error::Io(std::io::ErrorKind::ConnectionAborted.into()));
        let rx1 = rx1.map(Ok);
        let rx2 = rx2.map(Ok);

        let transport_l = Self::from_framed_pair(rx1, tx2);
        let transport_r = Self::from_framed_pair(rx2, tx1);
        (transport_l, transport_r)
    }

    pub fn split(
        self,
    ) -> (
        GenericStream<Result<RpcFrame>>,
        GenericSink<RpcFrame, Error>,
    ) {
        (self.input, self.output)
    }
}
