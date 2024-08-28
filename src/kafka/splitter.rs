use super::{KafkaConsumer, KafkaProducer};

pub struct KafkaSplitter {
    consumer: KafkaConsumer,
    producer: KafkaProducer,
}

impl KafkaSplitter {
    pub fn new(consumer: KafkaConsumer, producer: KafkaProducer) -> Self {
        Self { consumer, producer }
    }

    pub async fn run(
        self,
        origin_topic: &str,
        fallback_topic: &str,
        filtered_topic: &str,
    ) -> anyhow::Result<()> {
        todo!()
    }
}
