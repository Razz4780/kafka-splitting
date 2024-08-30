use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
};

use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use kafka_splitting::{kafka::KafkaProducer, ApiMessage};
use rdkafka::{
    message::{Header, OwnedHeaders},
    producer::FutureRecord,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Params {
    port: u16,
    #[serde(flatten)]
    producer_props: HashMap<String, String>,
}

#[tokio::main]
async fn main() {
    let params = envy::prefixed("PRODUCER_").from_env::<Params>().unwrap();

    let producer = KafkaProducer::new(params.producer_props).unwrap();

    let app = Router::new().route("/", post(handler)).with_state(producer);
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), params.port))
            .await
            .unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler(
    State(producer): State<KafkaProducer>,
    Json(mut message): Json<ApiMessage>,
) -> Result<Json<ApiMessage>, StatusCode> {
    let partition = producer
        .send(FutureRecord {
            topic: &message.topic,
            partition: message.partition,
            payload: message.payload.as_deref().map(str::as_bytes),
            key: message.key.as_deref().map(str::as_bytes),
            timestamp: message.timestamp,
            headers: message.headers.as_ref().map(|headers| {
                headers.iter().fold(
                    OwnedHeaders::new_with_capacity(headers.len()),
                    |acc, header| {
                        acc.insert(Header {
                            key: header.0,
                            value: header.1.as_ref(),
                        })
                    },
                )
            }),
        })
        .await
        .map_err(|error| {
            eprintln!("Failed to send message to Kafka: {error}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    message.partition.replace(partition);

    Ok(Json(message))
}
