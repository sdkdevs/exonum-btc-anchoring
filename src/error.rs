pub use details::error::Error as InternalError;
pub use handler::error::Error as HandlerError;
use bitcoinrpc::Error as RpcError;

/// Anchoring btc service Error type.
#[derive(Debug, Error)]
pub enum Error {
    /// Internal error
    Internal(InternalError),
    /// Handler error.
    Handler(HandlerError),
}

impl From<RpcError> for Error {
    fn from(err: RpcError) -> Error {
        Error::Internal(InternalError::Rpc(err))
    }
}
