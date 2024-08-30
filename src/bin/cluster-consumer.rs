use std::collections::HashMap;

use kafka_splitting::kafka::KafkaConsumer;
use rdkafka::{message::Headers, Message};
use serde::Deserialize;

#[derive(Deserialize)]
struct Params {
    #[serde(flatten)]
    consumer_props: HashMap<String, String>,
    topic: String,
}

#[tokio::main]
async fn main() {
    let params = envy::prefixed("CONSUMER_").from_env::<Params>().unwrap();

    let consumer = KafkaConsumer::new(params.consumer_props).unwrap();
    consumer.subscribe(params.topic).await.unwrap();

    loop {
        let msg = consumer.recv().await.unwrap();
        eprintln!(
            "CONSUMED MESSAGE: topic=({}) partition=({}) offset=({}) timestamp=({:?}) key=({:?}) headers=({:?}) payload=({:?})",
            msg.topic(),
            msg.partition(),
            msg.offset(),
            msg.timestamp().to_millis(),
            msg.key_view::<str>(),
            msg.headers().map(|headers| headers.iter().map(|header| (header.key, header.value.map(String::from_utf8_lossy))).collect::<Vec<_>>()),
            msg.payload_view::<str>(),
        )
    }
}
