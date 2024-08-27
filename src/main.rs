use std::str::FromStr;

use anyhow::Context;
use clap::{Parser, Subcommand};
use configuration::{KafkaSplittingConfiguration, SplitTarget};
use kube::{runtime::reflector::Lookup, Api, Client, Config, CustomResourceExt};

mod configuration;
mod crd;

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

    ResolveTarget {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Args { command } = Args::parse();
    match command {
        Command::GenerateCrds => {
            serde_yaml::to_writer(std::io::stdout(), &crd::MirrordKafkaClientConfig::crd())
                .with_context(|| {
                    format!(
                        "failed to output {} definition",
                        crd::MirrordKafkaClientConfig::kind(&())
                    )
                })?;
            println!("---");
            serde_yaml::to_writer(
                std::io::stdout(),
                &crd::MirrordKafkaSplittingTopicConsumer::crd(),
            )
            .with_context(|| {
                format!(
                    "failed to output {} definition",
                    crd::MirrordKafkaSplittingTopicConsumer::kind(&())
                )
            })?;
        }

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

        Command::ResolveTarget {
            configuration_namespace,
            api_version,
            kind,
            name,
            namespace,
            container_name,
            topic_id,
        } => {
            let config = Config::infer()
                .await
                .context("failed to infer kube config")?;
            let client = Client::try_from(config).context("failed to build kube config")?;
            let configuration = KafkaSplittingConfiguration::new(client, &configuration_namespace)
                .await
                .context("failed to intialize config watcher")?;

            let splitting_props = configuration
                .reolve_splitting_props(SplitTarget {
                    topic_id: &topic_id,
                    name: &name,
                    namespace: &namespace,
                    container_name: &container_name,
                    kind: &kind,
                    api_version: &api_version,
                })
                .context("failed to resolve splitting properties")?;

            println!("{splitting_props:?}")
        }
    }

    Ok(())
}
