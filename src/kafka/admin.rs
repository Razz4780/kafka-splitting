use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::Context;
use rdkafka::{
    admin::{AdminClient, AdminOptions, NewTopic},
    client::DefaultClientContext,
    config::FromClientConfig,
    error::IsError,
    ClientConfig,
};

pub struct KafkaAdminClient {
    client: Arc<AdminClient<DefaultClientContext>>,
}

impl KafkaAdminClient {
    pub fn new(props: HashMap<String, String>) -> anyhow::Result<Self> {
        let config = ClientConfig::from_iter(props);
        let client = AdminClient::from_config(&config)?;

        Ok(Self {
            client: Arc::new(client),
        })
    }

    pub async fn get_topic_partitions(&self, topic_name: &str) -> anyhow::Result<usize> {
        let client = self.client.clone();
        let owned_name = topic_name.to_string();

        let metadata = tokio::task::spawn_blocking(move || {
            client
                .inner()
                .fetch_metadata(Some(&owned_name), Some(Duration::from_secs(5)))
        })
        .await??;
        let topic_metadata = metadata
            .topics()
            .iter()
            .find(|metadata| metadata.name() == topic_name)
            .context("topic metadata not found in Kafka response")?;

        if let Some(err) = topic_metadata.error().filter(IsError::is_error) {
            anyhow::bail!("topic has error: {err:?} ({})", err as i32);
        }

        Ok(topic_metadata.partitions().len())
    }

    pub async fn create_topics<'a, I>(&self, topics: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = &'a NewTopic<'a>>,
    {
        let results = self
            .client
            .create_topics(
                topics,
                &AdminOptions::new().request_timeout(Some(Duration::from_secs(5))),
            )
            .await?;
        let errors = results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .map(|(name, error_code)| {
                format!("{} ({:?} {})", name, error_code, *error_code as isize)
            })
            .collect::<Vec<_>>();

        if !errors.is_empty() {
            let to_delete = results
                .iter()
                .filter_map(|result| result.as_deref().ok())
                .collect::<Vec<_>>();

            self.client
                .delete_topics(
                    &to_delete,
                    &AdminOptions::new().request_timeout(Some(Duration::from_secs(5))),
                )
                .await
                .inspect_err(|error| eprintln!("Failed to clean up topics: {error}"))
                .ok();

            anyhow::bail!("failed to create topics: {}", errors.join(", "));
        }

        Ok(())
    }
}
