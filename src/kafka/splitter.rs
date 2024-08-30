use anyhow::Context;
use rdkafka::{
    message::{BorrowedHeaders, BorrowedMessage},
    producer::FutureRecord,
    Message,
};

use super::{KafkaConsumer, KafkaProducer};

pub struct KafkaSplitter<F> {
    consumer: KafkaConsumer,
    producer: KafkaProducer,
    filter: F,
}

impl<F> KafkaSplitter<F>
where
    F: for<'a> FnMut(&BorrowedMessage<'a>) -> bool,
{
    pub fn new(consumer: KafkaConsumer, producer: KafkaProducer, filter: F) -> Self {
        Self {
            consumer,
            producer,
            filter,
        }
    }

    pub async fn run(
        mut self,
        origin_topic: &str,
        fallback_topic: &str,
        filtered_topic: &str,
    ) -> anyhow::Result<()> {
        self.consumer
            .subscribe(origin_topic.into())
            .await
            .context("failed to subscribe origin topic")?;

        loop {
            let message = self
                .consumer
                .recv()
                .await
                .context("failed to receive message")?;

            let topic = if (self.filter)(&message) {
                filtered_topic
            } else {
                fallback_topic
            };

            let record = FutureRecord {
                topic,
                partition: Some(message.partition()),
                payload: message.payload(),
                key: message.key(),
                timestamp: message.timestamp().to_millis(),
                headers: message.headers().map(BorrowedHeaders::detach),
            };

            if (self.filter)(&message) {
                self.producer
                    .send(record)
                    .await
                    .context("failed to pass received message")?;
            }
        }
    }
}
