use std::collections::HashMap;
use std::io;
use std::net::SocketAddr;
use std::ops::Not;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use fancy_regex::Regex;
use k8s_openapi::{api::apps::v1::Deployment, Resource};
use kafka_splitting::configuration::{KafkaSplittingConfiguration, SplitTarget};
use kafka_splitting::{
    crd, k8s_util,
    kafka::{KafkaAdminClient, KafkaConsumer, KafkaProducer, KafkaSplitter},
};
use kafka_splitting::{kafka, ApiMessage};
use kube::{runtime::reflector::Lookup, Api, Client, Config, CustomResourceExt};
use rand::Rng;
use rdkafka::admin::{NewTopic, TopicReplication};

#[derive(Parser)]
struct MainArgs {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    GenerateCrds,

    RunSplit(SplitArgs),

    PostMessage(PostMessageArgs),

    CreateTopic(CreateTopicArgs),
}

#[derive(Args)]
struct SplitArgs {
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
    filter: Regex,
}

#[derive(Args)]
struct PostMessageArgs {
    #[arg(long, short)]
    addr: SocketAddr,

    #[arg(long, short)]
    key: Option<String>,

    #[arg(long, short)]
    topic: String,

    #[arg(long, short)]
    partition: Option<i32>,

    #[arg(long)]
    timestamp: Option<i64>,

    #[arg(long, short)]
    header: Vec<String>,

    #[arg(long)]
    payload: Option<String>,
}

#[derive(Args)]
struct CreateTopicArgs {
    #[arg(long, short)]
    name: String,

    #[arg(long, short)]
    partitions: i32,

    #[arg(long, short)]
    bootstrap_servers: String,
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

async fn run_split(args: SplitArgs) -> anyhow::Result<()> {
    anyhow::ensure!(
        args.api_version == Deployment::API_VERSION && args.kind == Deployment::KIND,
        "invalid resource type, this tool currently handles only {} {}",
        Deployment::API_VERSION,
        Deployment::KIND
    );

    let config = Config::infer()
        .await
        .context("failed to infer kube config")?;
    let client = Client::try_from(config).context("failed to build kube config")?;

    let configuration =
        KafkaSplittingConfiguration::new(client.clone(), &args.configuration_namespace)
            .await
            .context("failed to intialize config watcher")?;

    let client_cloned = client.clone();
    let name = args.name.clone();
    let namespace = args.namespace.clone();
    let fetch_target_task = tokio::spawn(async move {
        let api: Api<Deployment> = Api::namespaced(client_cloned, &namespace);
        let target = api
            .get(&name)
            .await
            .context("failed to fetch target deployment")?;

        Ok::<_, anyhow::Error>((target, api))
    });

    let splitting_props = configuration
        .resolve_splitting_props(SplitTarget {
            topic_id: &args.topic_id,
            name: &args.name,
            namespace: &args.namespace,
            container_name: &args.container_name,
            kind: &args.kind,
            api_version: &args.api_version,
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

    let (target, api) = fetch_target_task
        .await
        .context("background task panicked")??;
    let container_spec = target
        .spec
        .as_ref()
        .and_then(|spec| spec.template.spec.as_ref())
        .and_then(|spec| {
            spec.containers
                .iter()
                .find(|container| container.name == args.container_name)
        })
        .context("failed to find target container in the pod template")?;
    eprintln!("Found target container");

    let topic_name = k8s_util::find_env_value(container_spec, &splitting_props.topic_name_env)
        .context("failed to resolve topic name in target spec")?;
    eprintln!("Resolved topic name {topic_name}");

    let group_id = k8s_util::find_env_value(container_spec, &splitting_props.group_id_env)
        .context("failed ot resolve topic name in target spec")?;
    eprintln!("Resolved group id {group_id}");
    consumer_props.insert("group.id".to_string(), group_id.into());

    let admin_client =
        KafkaAdminClient::new(admin_client_props).context("failed to build Kafka admin client")?;
    let consumer = KafkaConsumer::new(consumer_props).context("failed to build Kafka consumer")?;
    let producer = KafkaProducer::new(producer_props).context("failed to build Kafka producer")?;
    eprintln!("Created Kafka clients");

    let num_partitions: i32 = admin_client
        .get_topic_partitions(topic_name)
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
        &api,
        &target,
        &splitting_props.topic_name_env,
        &tmp_topic_fallback_name,
    )
    .await
    .context("failed to patch target deployment")?;

    k8s_util::wait_for_rollout_completion(api.clone(), &args.name, new_generation)
        .await
        .context("failed to wait until target rollout completes")?;

    KafkaSplitter::new(consumer, producer, kafka::regex_filter(args.filter))
        .run(
            topic_name,
            &tmp_topic_fallback_name,
            &tmp_topic_filtered_name,
        )
        .await
        .context("Kafka splitter failed")?;

    Ok(())
}

async fn post_message(args: PostMessageArgs) -> anyhow::Result<()> {
    let headers = args
        .header
        .into_iter()
        .map(|header| {
            let (key, value) = header
                .split_once(": ")
                .with_context(|| format!("invalid header {header}"))?;
            let value = value.is_empty().not().then(|| value.to_string());
            Ok::<_, anyhow::Error>((key.to_string(), value))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    let message = ApiMessage {
        topic: args.topic,
        partition: args.partition,
        payload: args.payload,
        key: args.key,
        timestamp: args.timestamp,
        headers: headers.is_empty().not().then_some(headers),
    };

    let response = reqwest::Client::new()
        .post(format!("http://{}", args.addr))
        .json(&message)
        .send()
        .await
        .context("failed to make request")?
        .error_for_status()
        .context("request rejected")?
        .json::<ApiMessage>()
        .await
        .context("failed to read response body")?;

    eprintln!("RESPONSE: {response:?}");

    Ok(())
}

async fn create_topic(args: CreateTopicArgs) -> anyhow::Result<()> {
    KafkaAdminClient::new([("bootstrap.servers".to_string(), args.bootstrap_servers)].into())
        .context("failed to build kafka client")?
        .create_topics([&NewTopic {
            name: &args.name,
            num_partitions: args.partitions,
            replication: TopicReplication::Fixed(1),
            config: Default::default(),
        }])
        .await
        .context("failed to create topic")?;

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let MainArgs { command } = MainArgs::parse();
    match command {
        Command::GenerateCrds => generate_crds(std::io::stdout()),
        Command::RunSplit(args) => run_split(args).await,
        Command::PostMessage(args) => post_message(args).await,
        Command::CreateTopic(args) => create_topic(args).await,
    }
}
