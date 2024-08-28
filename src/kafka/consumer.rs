use std::collections::HashMap;

use rdkafka::{
    config::FromClientConfig,
    consumer::{Consumer, StreamConsumer},
    ClientConfig,
};

pub struct KafkaConsumer {
    client: StreamConsumer,
}

impl KafkaConsumer {
    pub fn new(properties: HashMap<String, String>) -> anyhow::Result<Self> {
        let config = ClientConfig::from_iter(properties);
        let client = StreamConsumer::from_config(&config)?;

        Ok(Self { client })
    }
}
