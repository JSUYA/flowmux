//! IPC protocol between the flowmux GUI process and the `flowmux` CLI.
//!
//! Wire format: newline-delimited JSON over a Unix domain socket at
//! `$XDG_RUNTIME_DIR/flowmux.sock`. Each line is a complete [`Envelope`].
//!
//! The verb set mirrors cmux's documented socket API surface. We treat
//! verbs we have not implemented yet as `Error::Unimplemented` rather
//! than removing them, so the CLI shape stays stable while features
//! land.

pub mod protocol;
pub mod client;
pub mod server;

pub use protocol::{Envelope, Request, Response, RpcError};
