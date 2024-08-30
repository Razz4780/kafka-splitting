use std::{collections::HashMap, sync::Arc};

use anyhow::Context;
use rdkafka::{
    config::FromClientConfig,
    consumer::{Consumer, StreamConsumer},
    message::BorrowedMessage,
    ClientConfig,
};

pub struct KafkaConsumer {
    client: Arc<StreamConsumer>,
}

impl KafkaConsumer {
    pub fn new(properties: HashMap<String, String>) -> anyhow::Result<Self> {
        let config = ClientConfig::from_iter(properties);
        let client = StreamConsumer::from_config(&config)?;

        Ok(Self {
            client: Arc::new(client),
        })
    }

    pub async fn subscribe(&self, topic: String) -> anyhow::Result<()> {
        let client = self.client.clone();

        tokio::task::spawn_blocking(move || client.subscribe(&[&topic]))
            .await
            .context("background task panicked")??;

        Ok(())
    }

    pub async fn recv(&self) -> anyhow::Result<BorrowedMessage<'_>> {
        self.client
            .recv()
            .await
            .context("failed to receive message")
    }
}
