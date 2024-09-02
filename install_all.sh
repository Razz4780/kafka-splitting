#!/bin/bash

set -e

helm install \
    --set "listeners.client.protocol=PLAINTEXT" \
    kafka \
    oci://registry-1.docker.io/bitnamicharts/kafka

mirrord exec -f mirrord.json -- cargo run --bin kafka-splitting -- create-topic --name dummy-topic --partitions 3 --bootstrap-servers kafka.default.svc.cluster.local:9092

kubectl apply -f ./setup.yaml
