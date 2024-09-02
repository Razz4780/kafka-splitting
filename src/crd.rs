//! Definitions of [CustomResource]s specyfing Kafka splitting configuration in the cluster.
//! These resources are meant to be created by cluster admin.

use std::collections::BTreeMap;

use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Top-level admin-side configuration for Kafka splitting.
///
/// This resource describes topic name extraction/injection logic for setting up mirrord sessions that match its filters.
/// Resources of this kind should only be created in the operator's namespace.
/// Resources living outside operator's namespace will be ignored by this specific operator.
///
/// # Precedence in case of multiple matches
///
/// When multiple resource of this kind match the given mirrord session,
/// one must take precedence over all others. Otherwise, a conflict arises and the session is aborted.
///
/// Precedence resolution flow is as follows:
/// 1. Compare precedence of [`MirrordKafkaSplittingTopicConsumerSpec::filter`]
/// 2. In case [`MirrordKafkaSplittingTopicConsumerSpec::filter`]s' precedence is equal,
///    literal [`MirrordKafkaSplittingTopicConsumerSpec::topic_id`] takes precendence over regex-based.
/// 3. Conflict, abort session setup.
///
/// # Examples
///
/// ```yaml
/// filter:
///   apiVersion: "apps/v1"
///   kind: "Deployment"
///   namespace: "default"
///   name: "login-checker"
///   container: "login-checker"
/// topicId: "logins"
/// topicNameAccessEnv: "KAFKA_TOPIC_LOGINS"
/// groupIdAccessEnv: "KAFKA_LOGINS_CONSUMER_GROUP"
/// adminClientProperties: "admin-kafka-config"
/// producerProperties: "producer-kafka-config"
/// consumerProperties: "consumer-kafka-config"
/// ```
/// With above config:
/// 1. Session must target `deployment/login-checker/container/login-checker`
/// 2. Session must request Kafka split of topic with id `logins`
/// 3. Topic name will be fetched from variable `KAFKA_TOPIC_LOGINS` defined in the `login-checker` container spec.
/// 4. Topic name will be replaced in all occurrences of variable `KAFKA_TOPIC_LOGINS` across all containers.
/// 5. Consumer group id will be fetched from variable `KAFKA_LOGINS_CONSUMER_GROUP` defined in the `login-checker` container spec.
///
/// ```yaml
/// filter:
///   apiVersion: "argoproj.io/v1alpha1"
///   kind: "Rollout"
///   namespace: "default"
///   name: "re:login-checker-[0-9]"
///   container: "login-checker"
/// topicId: "re:logins-[0-9]"
/// topicNameAccessEnv: "KAFKA_TOPIC_LOGINS_{{ topic_id | trim_start_matches(pat=\"logins-\") }}_{{ resource_name | trim_start_matches(pat=\"login-checker-\") }}"
/// groupIdAccessEnv: "KAFKA_TOPIC_LOGINS_{{ resource_name | trim_start_matches(pat=\"login-checker-\") }}"
/// adminClientProperties: "admin-kafka-config"
/// producerProperties: "producer-kafka-config"
/// consumerProperties: "consumer-kafka-config"
/// ```
///
/// With above config, when targeting `rollout/login-checker-3/container/login-checker` with Kafka split request of topic with if `logins-8`:
/// 1. Topic name will be fetched from variable `KAFKA_TOPIC_LOGINS_8_3` defined in `login-checker` container spec.
/// 2. Topic name will be replaced in all occurrences of variable `KAFKA_TOPIC_LOGINS_8_3` across all containers.
/// 3. Consumer group id will be fetched from variable `KAFKA_LOGINS_CONSUMER_GROUP_3` defined in the `login-checker` container spec.
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "queues.mirrord.metalbear.co",
    version = "v1alpha",
    kind = "MirrordKafkaSplittingTopicConsumer",
    namespaced,
    printcolumn = r#"{"name":"topicId", "type":"string", "description":"ID of the consumed topic.", "jsonPath":".spec.topicId"}"#,
    printcolumn = r#"{"name":"topicNameAccessEnv", "type":"string", "description":"Name of environment variable where topic name can be altered.", "jsonPath":".spec.topicNameAccessEnv"}"#,
    printcolumn = r#"{"name":"groupIdAccessEnv", "type":"string", "description":"Name of environment variable from where consumer group id can be fetched.", "jsonPath":".spec.groupIdAccessEnv"}"#,
    printcolumn = r#"{"name":"adminProperties", "type":"string", "description":"Name of MirrordKafkaClientConfig to use when creating admin Kafka client.", "jsonPath":".spec.adminClientProperties"}"#,
    printcolumn = r#"{"name":"producerProperties", "type":"string", "description":"Name of MirrordKafkaClientConfig to use when creating producer Kafka client.", "jsonPath":".spec.producerProperties"}"#,
    printcolumn = r#"{"name":"consumerProperties", "type":"string", "description":"Name of MirrordKafkaClientConfig to use when creating consumer Kafka client.", "jsonPath":".spec.consumerProperties"}"#,

)]
#[serde(rename_all = "camelCase")]
pub struct MirrordKafkaSplittingTopicConsumerSpec {
    /// Filter to match against resource that is targeted by the mirrord session.
    pub filter: TopicConsumerFilter,

    /// Filter to match against topic id requested in Kafka split by the mirrord session.
    /// Value starting with `re:` should be interpreted as a regular expression to match against topic id requested in the mirrord session.
    ///
    /// # **Warning**
    ///
    /// Topic id is **not** topic name. It is a mirrord-specific identifier.
    pub topic_id: String,

    /// [tera](https://keats.github.io/tera/docs/) template for the name of the environment variable in the target resource [`Pod`](k8s_openapi::api::core::v1::Pod) template.
    /// When rendering, this template is provided with the followingcontext:
    /// 1. `topic_id` - requested topic id
    /// 2. `resource_name` - name of the targeted resource
    ///
    /// After rendering variable name, mirrord operator will:
    /// 1. Fetch Kafka topic name from this variable defined in the targeted container
    /// 2. Replace value of this variable across **all** containers in the target resource's pod template
    pub topic_name_access_env: String,

    /// [tera](https://keats.github.io/tera/docs/) template for the name of the environment variable in the target resource [`Pod`](k8s_openapi::api::core::v1::Pod) template.
    /// When rendering, this template is provided with the followingcontext:
    /// 1. `topic_id` - requested topic id
    /// 2. `resource_name` - name of the targeted resource
    ///
    /// After rendering variable name, mirrord operator will fetch consumer group id from this variable defined in the targeted container.
    pub group_id_access_env: String,

    /// Name of [`MirrordKafkaClientConfig`] resource to use for creating Kafka admin client.
    pub admin_client_properties: String,

    /// Name of [`MirrordKafkaClientConfig`] resource to use for creating Kafka consumer client.
    pub producer_properties: String,

    /// Name of [`MirrordKafkaClientConfig`] resource to use for creating Kafka producer client.
    pub consumer_properties: String,
}

/// Composable configuration for creating a Kafka client, used in [`MirrordKafkaSplittingTopicConsumer`].
/// Resources of this kind should only be created in the operator's namespace.
/// Resources living outside operator's namespace will be ignored.
///
/// # Example
///
/// Assuming we have following resources of this kind:
///
/// `base`:
/// ```yaml
/// properties:
///   client.id: "mirrord-operator"
///   security.protocol: "PLAINTEXT"
/// ```
///
/// `cluster-1`:
/// ```yaml
/// parent: "base"
/// properties:
///   bootstrap.servers: "kafka.default.svc.cluster.local:9092"
/// ```
///
/// `consumer-cluster-1`:
/// ```yaml
/// parent: "cluster-1"
/// properties:
///   group.id: "consumer-group-1"
/// ```
///
/// `consumer-cluster-1-no-client-id`:
/// ```yaml
/// parent: "consumer-cluster-1"
/// properties:
///   client.id: null
/// ```
///
/// `consumer-cluster-1` will resolve to following `.properties` file:
/// ```properties
/// client.id=mirrord-operator
/// security.protocol=PLAINTEXT
/// bootstrap.servers=kafka.default.svc.cluster.local:9092
/// group.id=consumer-group-1
/// ```
///
/// `consumer-cluster-1-no-client-id` will resolve to following `.properties` file:
/// ```properties
/// security.protocol=PLAINTEXT
/// bootstrap.servers=kafka.default.svc.cluster.local:9092
/// group.id=consumer-group-1
/// ```
#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "queues.mirrord.metalbear.co",
    version = "v1alpha",
    kind = "MirrordKafkaClientConfig",
    namespaced,
    printcolumn = r#"{"name":"parent", "type":"string", "description":"Name of parent resource.", "jsonPath":".spec.parent"}"#,
)]
#[serde(rename_all = "camelCase")]
pub struct MirrordKafkaClientConfigSpec {
    /// Parent of this config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,

    /// Properties to use when creating a Kafka client.
    ///
    /// To resolve final value:
    /// 1. Resolve value from parent [`MirrordKafkaClientConfig`].
    /// 2. Merge this config into it, replacing entries in case of key conflict.
    ///    Remove all entries with empty values.
    pub properties: BTreeMap<String, Option<String>>,
}

/// Filter for Kubernetes resources that provide a [`Pod`](k8s_openapi::api::core::v1::Pod) template.
/// When two filters match the same resource, precedence is determinted based on [`TopicConsumerFilter::name`] flavor (constant takes precedence over regex).
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicConsumerFilter {
    /// API version of the resource, e.g `apps/v1`.
    pub api_version: String,
    /// Kind of the resource, e.g `Deployment`.
    pub kind: String,
    /// Namespace of the resource, e.g. `prod-mirror`.
    pub namespace: String,
    /// Name of the resource, e.g. `my-deployment` or `re:my-deployment.*`.
    /// Value starting with `re:` should be interpreted as a regular expression to match against resource name.
    pub name: String,
    /// Name of the consuming container.
    pub container: String,
}
