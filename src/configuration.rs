use std::{cmp::Ordering, collections::HashMap, fmt, sync::Arc};

use anyhow::Context as _;
use fancy_regex::Regex;
use futures::StreamExt;
use kube::{
    runtime::{self, watcher::Event, WatchStreamExt},
    Api, Client, Resource,
};
use serde::de::DeserializeOwned;
use tera::{Context, Tera};
use tokio::sync::watch;

use crate::crd::{self, TopicConsumerFilter};

#[derive(Clone)]
pub struct KafkaSplittingConfiguration {
    topic_consumers: watch::Receiver<ConfigResourceMap<crd::MirrordKafkaSplittingTopicConsumer>>,
    client_configs: watch::Receiver<ConfigResourceMap<crd::MirrordKafkaClientConfig>>,
}

impl KafkaSplittingConfiguration {
    pub async fn new(client: Client, namespace: &str) -> anyhow::Result<Self> {
        let mut topic_consumers = watch::channel(Default::default());
        tokio::spawn(
            ConfigCrdWatcher::new(
                Api::<crd::MirrordKafkaSplittingTopicConsumer>::namespaced(
                    client.clone(),
                    namespace,
                ),
                topic_consumers.0,
            )
            .run(),
        );

        let mut client_configs = watch::channel(Default::default());
        tokio::spawn(
            ConfigCrdWatcher::new(
                Api::<crd::MirrordKafkaClientConfig>::namespaced(client, namespace),
                client_configs.0,
            )
            .run(),
        );

        if topic_consumers.1.changed().await.is_err() {
            anyhow::bail!(
                "background task watching {} is dead",
                crd::MirrordKafkaSplittingTopicConsumer::plural(&())
            );
        }
        if client_configs.1.changed().await.is_err() {
            anyhow::bail!(
                "background task watching {} is dead",
                crd::MirrordKafkaClientConfig::plural(&())
            );
        }

        Ok(Self {
            topic_consumers: topic_consumers.1,
            client_configs: client_configs.1,
        })
    }

    pub async fn background_task_dead(&mut self) -> anyhow::Error {
        loop {
            tokio::select! {
                Err(..) = self.topic_consumers.changed() => {
                    break anyhow::anyhow!(
                        "background task watching {} is dead",
                        crd::MirrordKafkaSplittingTopicConsumer::plural(&()),
                    );
                },

                Err(..) = self.client_configs.changed() => {
                    break anyhow::anyhow!(
                        "background task watching {} is dead",
                        crd::MirrordKafkaClientConfig::plural(&()),
                    );
                },

                else => {},
            }
        }
    }

    pub fn resolve_client_config(&self, name: &str) -> anyhow::Result<HashMap<String, String>> {
        let configs = self.client_configs.borrow().clone();
        let mut children: Vec<&crd::MirrordKafkaClientConfig> = Default::default();
        let mut acc: HashMap<&str, &str>;
        let mut current_name = name;

        loop {
            let config = configs.get(name).with_context(|| {
                format!(
                    "{} {current_name} does not exist",
                    crd::MirrordKafkaClientConfig::kind(&())
                )
            })?;

            match config.spec.parent.as_ref() {
                Some(parent) => {
                    children.push(config);
                    current_name = parent
                }
                None => {
                    acc = config
                        .spec
                        .properties
                        .iter()
                        .filter_map(|(key, value)| Some((key.as_str(), value.as_deref()?)))
                        .collect();
                    break;
                }
            }
        }

        while let Some(child) = children.pop() {
            for (key, value) in child.spec.properties.iter() {
                match value.as_ref() {
                    Some(value) => {
                        acc.insert(key, value);
                    }
                    None => {
                        acc.remove(key.as_str());
                    }
                }
            }
        }

        Ok(acc
            .into_iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect())
    }

    pub fn reolve_target_env(
        &self,
        split_target: SplitTarget<'_>,
    ) -> anyhow::Result<(String, String)> {
        let mut matching: Vec<&crd::MirrordKafkaSplittingTopicConsumer> = Default::default();

        let consumers = self.topic_consumers.borrow().clone();
        for consumer in consumers.values() {
            if !consumer.spec.filter.matches(&split_target) {
                continue;
            }

            let topic_id_matches = RegexOrLiteral::try_from(consumer.spec.topic_id.as_str())
                .map(|rl| rl.matches(split_target.topic_id))
                .unwrap_or(false);
            if !topic_id_matches {
                continue;
            }

            matching.push(consumer);
        }

        let winner = matching
            .iter()
            .max_by(|c1, c2| c1.cmp_precedence(c2))
            .copied()
            .with_context(|| {
                format!(
                    "no matching {} found",
                    crd::MirrordKafkaSplittingTopicConsumer::kind(&()),
                )
            })?;

        if matching.iter().any(|consumer| {
            let winner_ptr: *const _ = winner;
            let consumer_ptr: *const _ = *consumer;
            consumer.cmp_precedence(winner).is_eq() && winner_ptr != consumer_ptr
        }) {
            anyhow::bail!(
                "conflicting {} found",
                crd::MirrordKafkaSplittingTopicConsumer::kind(&())
            );
        }

        let mut context = Context::new();
        context.insert("topic_id", split_target.topic_id);
        context.insert("resource_name", split_target.name);
        let topic_name_env = Tera::one_off(&winner.spec.topic_name_access_env, &context, false)
            .context("failed to render topic name environment variable name")?;
        let group_id_env = Tera::one_off(&winner.spec.group_id_access_env, &context, false)
            .context("failed to render groupd id environment variable name")?;

        Ok((topic_name_env, group_id_env))
    }
}

type ConfigResourceMap<R> = Arc<HashMap<String, R>>;

struct ConfigCrdWatcher<R> {
    api: Api<R>,
    tx: watch::Sender<ConfigResourceMap<R>>,
    init_buffer: HashMap<String, R>,
}

impl<R> ConfigCrdWatcher<R>
where
    R: 'static + Resource + Clone + fmt::Debug + Send + Sync + DeserializeOwned,
{
    fn new(api: Api<R>, tx: watch::Sender<ConfigResourceMap<R>>) -> Self {
        Self {
            api,
            tx,
            init_buffer: Default::default(),
        }
    }

    fn handle_events(&mut self, event: Event<R>) {
        match event {
            Event::Apply(resource) => {
                let Some(name) = resource.meta().name.clone() else {
                    return;
                };

                let mapping = self.tx.borrow().clone();
                let mut mapping = mapping.as_ref().clone();
                mapping.insert(name, resource);
                let _ = self.tx.send(Arc::new(mapping));
            }

            Event::Delete(resource) => {
                let Some(name) = resource.meta().name.as_ref() else {
                    return;
                };

                let mapping = self.tx.borrow().clone();
                let mut mapping = mapping.as_ref().clone();
                mapping.remove(name);
                let _ = self.tx.send(Arc::new(mapping));
            }

            Event::Init => {}

            Event::InitApply(resource) => {
                let Some(name) = resource.meta().name.clone() else {
                    return;
                };

                self.init_buffer.insert(name, resource);
            }

            Event::InitDone => {
                let mapping = std::mem::take(&mut self.init_buffer);
                let _ = self.tx.send(Arc::new(mapping));
            }
        }
    }

    async fn run(mut self) {
        let mut stream =
            Box::pin(runtime::watcher(self.api.clone(), Default::default()).default_backoff());
        loop {
            tokio::select! {
                item = stream.next() => match item {
                    Some(Ok(event)) => {
                        self.handle_events(event);
                    },
                    Some(Err(..)) => {
                    },
                    None => {
                        stream = Box::pin(
                            runtime::watcher(self.api.clone(), Default::default())
                                .default_backoff()
                        );
                    }
                },

                _ = self.tx.closed() => {
                    break;
                },
            }
        }
    }
}

enum RegexOrLiteral<'a> {
    Literal(&'a str),
    Regex(Regex),
}

impl<'a> RegexOrLiteral<'a> {
    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Literal(literal) => *literal == name,
            Self::Regex(regex) => regex.is_match(name).unwrap_or(false),
        }
    }
}

impl<'a> TryFrom<&'a str> for RegexOrLiteral<'a> {
    type Error = anyhow::Error;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        match value.strip_prefix("re:") {
            Some(re) => {
                let regex = Regex::new(re).context("failed to parse regular expression")?;
                Ok(Self::Regex(regex))
            }
            None => Ok(Self::Literal(value)),
        }
    }
}

pub struct SplitTarget<'a> {
    pub topic_id: &'a str,
    pub name: &'a str,
    pub namespace: &'a str,
    pub container_name: &'a str,
    pub kind: &'a str,
    pub api_version: &'a str,
}

trait TopicConsumerFilterExt {
    fn matches(&self, split_target: &SplitTarget<'_>) -> bool;

    fn cmp_precedence(&self, other: &Self) -> Ordering;
}

impl TopicConsumerFilterExt for TopicConsumerFilter {
    fn matches(&self, split_target: &SplitTarget<'_>) -> bool {
        self.api_version == split_target.api_version
            && self.kind == split_target.kind
            && self.namespace == split_target.namespace
            && self.container == split_target.container_name
            && RegexOrLiteral::try_from(self.name.as_str())
                .map(|rl| rl.matches(split_target.name))
                .unwrap_or(false)
    }

    fn cmp_precedence(&self, other: &Self) -> Ordering {
        match (self.name.starts_with("re:"), other.name.starts_with("re:")) {
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            _ => Ordering::Equal,
        }
    }
}

trait TopicConsumerExt {
    fn cmp_precedence(&self, other: &Self) -> Ordering;
}

impl TopicConsumerExt for crd::MirrordKafkaSplittingTopicConsumer {
    fn cmp_precedence(&self, other: &Self) -> Ordering {
        match self.spec.filter.cmp_precedence(&other.spec.filter) {
            Ordering::Equal => match (
                self.spec.topic_id.starts_with("re:"),
                other.spec.topic_id.starts_with("re:"),
            ) {
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                _ => Ordering::Equal,
            },
            other => other,
        }
    }
}
