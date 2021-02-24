use std::{
    io::{Read, Write},
    marker::PhantomData,
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    error::Error,
    rpc::{RequestId, RpcFrame},
};

pub trait RpcSerializer: Send + Sync + 'static {
    fn serialize_to<T: Serialize, W: Write>(writer: W, value: T) -> Result<(), Error>;
    fn deserialize_from<T: DeserializeOwned, R: Read>(reader: R) -> Result<T, Error>;

    fn serialize_size_hint<T: Serialize>(_value: &T) -> usize {
        120
    }
}

pub enum BincodeSerializer {}

impl RpcSerializer for BincodeSerializer {
    fn serialize_to<T: Serialize, W: Write>(writer: W, value: T) -> Result<(), Error> {
        bincode::serialize_into(writer, &value).map_err(Into::into)
    }

    fn deserialize_from<T: DeserializeOwned, R: Read>(reader: R) -> Result<T, Error> {
        bincode::deserialize_from(reader).map_err(Into::into)
    }

    fn serialize_size_hint<T: Serialize>(value: &T) -> usize {
        bincode::serialized_size(value).unwrap_or(0) as usize
    }
}

pub enum JsonSerializer {}

impl RpcSerializer for JsonSerializer {
    fn serialize_to<T: Serialize, W: Write>(writer: W, value: T) -> Result<(), Error> {
        serde_json::to_writer(writer, &value)
            .map_err(Box::new)
            .map_err(Into::into)
    }

    fn deserialize_from<T: DeserializeOwned, R: Read>(reader: R) -> Result<T, Error> {
        serde_json::from_reader(reader)
            .map_err(Box::new)
            .map_err(Into::into)
    }
}

pub struct SerializedFrame<S: RpcSerializer>(Bytes, PhantomData<S>);

impl<S: RpcSerializer> SerializedFrame<S> {
    pub fn new(buf: impl Into<Bytes>) -> Self {
        Self(buf.into(), PhantomData)
    }

    pub fn inner(&self) -> Bytes {
        self.0.clone()
    }
}

impl<S: RpcSerializer> From<Bytes> for SerializedFrame<S> {
    fn from(value: Bytes) -> Self {
        Self::new(value)
    }
}

impl<S: RpcSerializer> From<BytesMut> for SerializedFrame<S> {
    fn from(value: BytesMut) -> Self {
        Self::new(value)
    }
}

impl<S: RpcSerializer> From<SerializedFrame<S>> for Bytes {
    fn from(value: SerializedFrame<S>) -> Self {
        value.0
    }
}

impl<T: Serialize + DeserializeOwned, S: RpcSerializer> RpcFrame<T> for SerializedFrame<S> {
    fn from_parts(id: RequestId, data: T) -> Result<Self, Error> {
        let cap = 8 + S::serialize_size_hint(&data);
        trace!("init with cap = {}", cap);
        let mut buf = BytesMut::with_capacity(cap);
        buf.put_u64(id.0);
        let mut writer = buf.writer();
        S::serialize_to(&mut writer, &data)?;
        let buf = writer.into_inner();
        trace!("fini with len = {}, cap = {}", buf.len(), buf.capacity());
        Ok(Self(buf.freeze(), PhantomData))
    }

    fn get_id(&self) -> RequestId {
        RequestId(self.0.clone().get_u64())
    }

    fn get_data(self) -> Result<T, Error> {
        S::deserialize_from(&self.0[8..])
    }
}
