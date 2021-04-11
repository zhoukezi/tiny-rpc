#[doc(hidden)]
pub mod re_export {
    pub extern crate serde;

    pub use std::{
        clone::Clone,
        convert::Into,
        marker::{Send, Sync},
        stringify,
        sync::Arc,
    };

    pub use futures::{
        future::{BoxFuture, FutureExt},
        stream::BoxStream,
    };
    pub use serde_derive::{Deserialize, Serialize};
    pub use tracing::Instrument;

    pub use crate::{
        error::{Error, ProtocolError, Result},
        io::{IdGenerator, RpcFrame, Transport},
        rpc::{Client, ClientDriverHandle, Server},
    };
}

use std::collections::HashMap;

use futures::{
    channel::{mpsc, oneshot},
    future::{select, BoxFuture, Either},
    stream::BoxStream,
    FutureExt, SinkExt, StreamExt,
};
use tracing::Instrument;

use crate::{
    error::{Error, ProtocolError, Result},
    io::{Id, RpcFrame, Transport},
};

pub trait Server: Clone + Send + Sync + 'static {
    fn make_response(self, req: RpcFrame) -> BoxFuture<'static, Result<RpcFrame>>;
    fn serve(self, transport: Transport) -> BoxStream<'static, BoxFuture<'static, ()>> {
        trace!("server start");

        let (mut recv, mut send) = transport.split();
        let (spawner_tx, spawner_rx) = mpsc::unbounded::<BoxFuture<'static, ()>>();
        let spawner = spawner_tx.clone();
        let serve_fut = async move {
            let (tx, mut rx) = mpsc::unbounded::<RpcFrame>();
            let mut fut = select(recv.next(), rx.next());
            loop {
                match fut.await {
                    Either::Left((Some(req_frame), r)) => {
                        let id = req_frame
                            .as_ref()
                            .map(|f| f.id().unwrap_or(Id::NULL))
                            .unwrap_or(Id::NULL);
                        trace!("accept request {}", id);

                        let span = info_span!("server", %id);
                        let tx = tx.clone();
                        let this = self.clone();
                        let rsp_fut = async move {
                            let req_frame = req_frame?;
                            let rsp_frame = this.make_response(req_frame).await?; // TODO send server fail
                            tx.unbounded_send(rsp_frame).map_err(|_| Error::Driver)?;
                            Ok::<_, Error>(())
                        }
                        .map(|r: Result<()>| log_error("responser", r))
                        .instrument(span)
                        .boxed();
                        spawner
                            .unbounded_send(rsp_fut)
                            .map_err(|_| Error::Spawner)?;
                        fut = select(recv.next(), r);
                    }
                    Either::Right((Some(rsp_frame), r)) => {
                        let id = rsp_frame.id().expect("misformed outgoing frame");
                        trace!("finish request {}", id);

                        send.send(rsp_frame).await?;
                        fut = select(r, rx.next());
                    }
                    _ => {
                        // None is returned from client or remote. Stop driver.
                        trace!("server stop");
                        break Ok(());
                    }
                }
            }
        };
        spawner_tx
            .unbounded_send(Box::pin(
                serve_fut
                    .map(|r: Result<()>| log_error("server driver", r))
                    .boxed(),
            ))
            .expect("infallible: unbounded mpsc");
        Box::pin(spawner_rx)
    }
}

#[derive(Clone)]
pub struct ClientDriverHandle {
    sender: mpsc::UnboundedSender<(RpcFrame, oneshot::Sender<Result<RpcFrame>>)>,
}

pub trait Client: Sized {
    fn from_handle(handle: ClientDriverHandle) -> Self;
    fn handle(&self) -> &ClientDriverHandle;
    fn new(transport: Transport) -> (Self, BoxFuture<'static, ()>) {
        let (mut recv, mut send) = transport.split();
        let (tx, mut rx) = mpsc::unbounded::<(RpcFrame, oneshot::Sender<Result<RpcFrame>>)>();
        let dispatcher_fut = async move {
            trace!("dispatcher start");

            let mut fut = select(recv.next(), rx.next());
            let mut req_map = HashMap::<Id, oneshot::Sender<Result<RpcFrame>>>::new();
            loop {
                match fut.await {
                    Either::Left((Some(rsp_frame), r)) => {
                        // recv response from server

                        let rsp_frame = rsp_frame?;
                        let id = rsp_frame.id()?;
                        trace!("finish request {}", id);

                        if let Some(handler) = req_map.remove(&id) {
                            if handler.send(Ok(rsp_frame)).is_err() {
                                debug!("respond to canceled request: {}", id);
                            }
                        } else {
                            break Err(ProtocolError::MartianResponse.into());
                        }
                        fut = select(recv.next(), r);
                    }
                    Either::Right((Some((req_frame, rsp_handler)), r)) => {
                        let id = req_frame.id().expect("misformed outgoing frame");
                        trace!("begin request {}", id);

                        if req_map.insert(id, rsp_handler).is_some() {
                            panic!("id duplication: {}", id);
                        }
                        send.send(req_frame).await?;
                        fut = select(r, rx.next());
                    }
                    _ => {
                        // None is returned from client or remote. Stop driver.
                        trace!("dispatcher stop");
                        break Ok(());
                    }
                }
            }
        };
        let handle = ClientDriverHandle { sender: tx };
        let client = Self::from_handle(handle);
        (
            client,
            dispatcher_fut
                .map(|r: Result<()>| log_error("client driver", r))
                .boxed(),
        )
    }

    fn make_request(&self, req: RpcFrame) -> BoxFuture<'static, Result<RpcFrame>> {
        let sender = self.handle().sender.clone();
        let fut = async move {
            let (handler_tx, handler_rx) = oneshot::channel();
            sender
                .unbounded_send((req, handler_tx))
                .map_err(|_| Error::Driver)?;
            handler_rx.await.map_err(|_| Error::Driver)?
        };
        fut.boxed()
    }
}

fn log_error(context: &'static str, r: Result<()>) {
    if let Err(e) = r {
        error!("{}: {}", context, e);
    }
}
