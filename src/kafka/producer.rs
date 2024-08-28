use std::collections::HashMap;

use rdkafka::{config::FromClientConfig, producer::FutureProducer, ClientConfig};

pub struct KafkaProducer {
    client: FutureProducer,
}

impl KafkaProducer {
    pub fn new(properties: HashMap<String, String>) -> anyhow::Result<Self> {
        let config = ClientConfig::from_iter(properties);
        let client = FutureProducer::from_config(&config)?;

        Ok(Self { client })
    }
}
