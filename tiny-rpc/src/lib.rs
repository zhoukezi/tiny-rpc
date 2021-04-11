// required to use proc macros inside this crate
extern crate self as tiny_rpc;
#[macro_use]
extern crate tracing;

pub mod error;
pub mod io;
pub mod rpc;
#[cfg(test)]
pub mod test;

pub use tiny_rpc_macros::rpc_define;
