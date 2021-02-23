#![feature(never_type)]
#![feature(result_flattening)]
#![feature(async_closure)]

// required to use define_rpc macro inside this crate
extern crate self as tiny_rpc;
#[macro_use]
extern crate log;

pub mod error;
pub mod io;
pub mod rpc;
pub mod serialize;
#[cfg(test)]
pub mod test;

pub use tiny_rpc_macros::rpc_define;
