FROM rust:latest

COPY ./ ./

RUN cargo build --release

RUN mkdir /app && mv ./target/release/cluster-* /app
