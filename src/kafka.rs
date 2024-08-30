mod admin;
mod consumer;
mod producer;
mod splitter;

use core::str;

pub use admin::KafkaAdminClient;
pub use consumer::KafkaConsumer;
use fancy_regex::Regex;
pub use producer::KafkaProducer;
use rdkafka::{
    message::{BorrowedMessage, Headers},
    Message,
};
pub use splitter::KafkaSplitter;

pub fn regex_filter(regex: Regex) -> impl FnMut(&BorrowedMessage<'_>) -> bool {
    move |message| {
        let Some(headers) = message.headers() else {
            return false;
        };

        for header in headers.iter() {
            let key = header.key;
            let value = header
                .value
                .and_then(|value| str::from_utf8(value).ok())
                .unwrap_or_default();
            let to_match = format!("{key}: {value}");
            if regex.is_match(&to_match).unwrap_or_default() {
                return true;
            }
        }

        false
    }
}
