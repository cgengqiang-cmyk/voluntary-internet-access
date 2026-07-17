//! Privileged TUN helper boundary.
//!
//! The helper accepts a small, versioned, authenticated protocol over a local
//! transport.  Callers can provide configuration *contents*, but never an
//! executable path, configuration path, service name, socket path, or command.
//! All privileged files are selected from [`HelperLayout::installed`].

mod config;
mod error;
mod ipc;
mod layout;
mod protocol;
mod runtime;

pub use error::{HelperError, HelperResult};
pub use ipc::{HelperClient, run_server};
pub use layout::{HelperLayout, PIPE_NAME, SOCKET_PATH};
pub use protocol::{
    HELPER_PROTOCOL_VERSION, HelperOperation, HelperRequest, HelperResponse, HelperResponseBody,
    HelperStatus,
};
