use std::{io, str::FromStr};

use anyhow::Context;
use clap::{Parser, Subcommand};
use configuration::{KafkaSplittingConfiguration, SplitTarget};
use k8s_openapi::{api::apps::v1::Deployment, Resource};
use kafka::{KafkaAdminClient, KafkaConsumer, KafkaProducer, KafkaSplitter};
use kube::{runtime::reflector::Lookup, Api, Client, Config, CustomResourceExt};
use rand::Rng;
use rdkafka::{
    admin::{NewTopic, TopicReplication},
    message::Headers,
    Message,
};

mod configuration;
mod crd;
mod k8s_util;
mod kafka;

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    GenerateCrds,

    ListAll {
        #[arg(short, long)]
        namespace: String,
        kind: ConfigKind,
    },

    Split {
        #[arg(short, long)]
        configuration_namespace: String,
        #[arg(long)]
        api_version: String,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        namespace: String,
        #[arg(long)]
        container_name: String,
        #[arg(long)]
        topic_id: String,
        #[arg(long)]
        print_only: bool,
    },
}

#[derive(Clone, Copy)]
enum ConfigKind {
    ClientConfig,
    TopicConsumer,
}

impl FromStr for ConfigKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == crd::MirrordKafkaClientConfig::plural(&()) {
            Ok(Self::ClientConfig)
        } else if s == crd::MirrordKafkaSplittingTopicConsumer::plural(&()) {
            Ok(Self::TopicConsumer)
        } else {
            anyhow::bail!(
                "expected one of {}, {}",
                crd::MirrordKafkaClientConfig::plural(&()),
                crd::MirrordKafkaSplittingTopicConsumer::plural(&())
            )
        }
    }
}

fn generate_crds<W: io::Write>(mut writer: W) -> anyhow::Result<()> {
    serde_yaml::to_writer(&mut writer, &crd::MirrordKafkaClientConfig::crd()).with_context(
        || {
            format!(
                "failed to generate {} definition",
                crd::MirrordKafkaClientConfig::kind(&())
            )
        },
    )?;

    writer.write_all(b"---\n")?;

    serde_yaml::to_writer(
        std::io::stdout(),
        &crd::MirrordKafkaSplittingTopicConsumer::crd(),
    )
    .with_context(|| {
        format!(
            "failed to generate {} definition",
            crd::MirrordKafkaSplittingTopicConsumer::kind(&())
        )
    })?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args { command } = Args::parse();
    match command {
        Command::GenerateCrds => generate_crds(std::io::stdout())?,

        Command::ListAll { kind, namespace } => {
            let config = Config::infer()
                .await
                .context("failed to infer kube config")?;
            let client = Client::try_from(config).context("failed to build kube client")?;

            match kind {
                ConfigKind::ClientConfig => {
                    let api = Api::<crd::MirrordKafkaClientConfig>::namespaced(client, &namespace);
                    let results = api
                        .list(&Default::default())
                        .await
                        .with_context(|| {
                            format!(
                                "failed to list {}",
                                crd::MirrordKafkaClientConfig::plural(&())
                            )
                        })?
                        .items;
                    for result in results {
                        println!("NAME=({:?}) SPEC=({:?})", result.metadata.name, result.spec);
                    }
                }

                ConfigKind::TopicConsumer => {
                    let api = Api::<crd::MirrordKafkaSplittingTopicConsumer>::namespaced(
                        client, &namespace,
                    );
                    let results = api
                        .list(&Default::default())
                        .await
                        .with_context(|| {
                            format!(
                                "failed to list {}",
                                crd::MirrordKafkaClientConfig::plural(&())
                            )
                        })?
                        .items;
                    for result in results {
                        println!("NAME=({:?}) SPEC={:?}", result.metadata.name, result.spec);
                    }
                }
            }
        }

        Command::Split {
            configuration_namespace,
            api_version,
            kind,
            name,
            namespace,
            container_name,
            topic_id,
            print_only,
        } => {
            let config = Config::infer()
                .await
                .context("failed to infer kube config")?;
            let client = Client::try_from(config).context("failed to build kube config")?;
            let configuration =
                KafkaSplittingConfiguration::new(client.clone(), &configuration_namespace)
                    .await
                    .context("failed to intialize config watcher")?;

            anyhow::ensure!(
                api_version == Deployment::API_VERSION && kind == Deployment::KIND,
                "invalid resource type, this tool currently handles only {} {}",
                Deployment::API_VERSION,
                Deployment::KIND
            );

            let deployment_api: Api<Deployment> = Api::namespaced(client, &namespace);

            let target = deployment_api
                .get(&name)
                .await
                .context("failed to fetch target deployment")?;
            eprintln!("Found target deployment");

            let container_spec = target
                .spec
                .as_ref()
                .and_then(|spec| spec.template.spec.as_ref())
                .and_then(|spec| {
                    spec.containers
                        .iter()
                        .find(|container| container.name == container_name)
                })
                .context("failed to find target container in the pod template")?;
            eprintln!("Found target container");

            let splitting_props = configuration
                .resolve_splitting_props(SplitTarget {
                    topic_id: &topic_id,
                    name: &name,
                    namespace: &namespace,
                    container_name: &container_name,
                    kind: &kind,
                    api_version: &api_version,
                })
                .context("failed to resolve splitting properties")?;
            eprintln!("Resolved splitting props: {splitting_props:?}");

            let admin_client_props = configuration
                .resolve_client_config(&splitting_props.admin_client_config)
                .context("failed to resolve Kafka admin client properties")?;
            eprintln!("Resolved admin client properties");

            let producer_props = configuration
                .resolve_client_config(&splitting_props.producer_config)
                .context("failed to resolve Kafka producer properties")?;
            eprintln!("Resolved producer properties");

            let mut consumer_props = configuration
                .resolve_client_config(&splitting_props.consumer_config)
                .context("failed to resolve Kafka consumer properties")?;
            eprintln!("Resolved consumer properties");

            let topic_name = container_spec
                .env
                .iter()
                .flatten()
                .find(|env_var| env_var.name == splitting_props.topic_name_env)
                .and_then(|env_var| env_var.value.as_ref())
                .with_context(|| {
                    format!(
                        "failed to find {} value in the target container spec",
                        splitting_props.topic_name_env
                    )
                })?;
            eprintln!("Resolved topic name {topic_name}");

            let group_id = container_spec
                .env
                .iter()
                .flatten()
                .find(|env_var| env_var.name == splitting_props.group_id_env)
                .and_then(|env_var| env_var.value.as_ref())
                .with_context(|| {
                    format!(
                        "failed to find {} value in the target container spec",
                        splitting_props.group_id_env
                    )
                })?;
            eprintln!("Resolved group id {group_id}");
            consumer_props.insert("group.id".to_string(), group_id.into());

            let admin_client = KafkaAdminClient::new(admin_client_props)
                .context("failed to build Kafka admin client")?;
            let consumer =
                KafkaConsumer::new(consumer_props).context("failed to build Kafka consumer")?;
            let producer =
                KafkaProducer::new(producer_props).context("failed to build Kafka producer")?;
            eprintln!("Created Kafka clients");

            let num_partitions: i32 = admin_client
                .get_topic_partitions(&topic_name)
                .await
                .context("failed to get topic partitions")?
                .try_into()
                .context("topic to split has too many partitions")?;

            let tmp_topic_fallback_name = format!(
                "{topic_name}-mirrord-fallback{}",
                rand::thread_rng().gen::<u32>()
            );
            let tmp_topic_filtered_name =
                format!("{topic_name}-mirrord-{}", rand::thread_rng().gen::<u32>());
            eprintln!("Splitting will create temporary topics {tmp_topic_filtered_name} and {tmp_topic_fallback_name}, each with {num_partitions} partitions");

            if print_only {
                eprintln!("Requested printing only, exiting");
                return Ok(());
            }

            admin_client
                .create_topics([
                    &NewTopic {
                        name: &tmp_topic_fallback_name,
                        num_partitions,
                        replication: TopicReplication::Fixed(1),
                        config: Default::default(),
                    },
                    &NewTopic {
                        name: &tmp_topic_filtered_name,
                        num_partitions,
                        replication: TopicReplication::Fixed(1),
                        config: Default::default(),
                    },
                ])
                .await
                .context("failed to create temporary topics")?;
            eprintln!("Temporary topics created");

            let new_generation = k8s_util::patch_env_name(
                &deployment_api,
                &target,
                &splitting_props.topic_name_env,
                &tmp_topic_fallback_name,
            )
            .await
            .context("failed to patch target deployment")?;

            k8s_util::wait_for_rollout_completion(deployment_api.clone(), &name, new_generation)
                .await
                .context("failed to wait until target rollout completes")?;

            KafkaSplitter::new(consumer, producer, |message| {
                message
                    .headers()
                    .map(|headers| headers.iter().any(|header| header.key.contains("mirrord")))
                    .unwrap_or(false)
            })
            .run(
                &topic_name,
                &tmp_topic_fallback_name,
                &tmp_topic_filtered_name,
            )
            .await
            .context("Kafka splitter failed")?;
        }
    }

    Ok(())
}
