mod basic;
mod full;

pub use basic::BasicMessage;
pub use full::FullMessage;

use serde::Serialize;

use rustlog_storage::message::StructuredMessage;

pub trait ResponseMessage<'a>: Sized + Send + Serialize + Unpin {
    fn from_structured(msg: &'a StructuredMessage<'a>) -> anyhow::Result<Self>;
}
