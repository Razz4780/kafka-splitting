use std::collections::HashMap;

use rdkafka::{
    config::FromClientConfig,
    producer::{FutureProducer, FutureRecord},
    ClientConfig,
};

pub struct KafkaProducer {
    client: FutureProducer,
}

impl KafkaProducer {
    pub fn new(properties: HashMap<String, String>) -> anyhow::Result<Self> {
        let config = ClientConfig::from_iter(properties);
        let client = FutureProducer::from_config(&config)?;

        Ok(Self { client })
    }

    pub async fn send(&self, message: FutureRecord<'_, [u8], [u8]>) -> anyhow::Result<()> {
        self.client
            .send(message, None)
            .await
            .map_err(|(error, _)| error)?;

        Ok(())
    }
}
