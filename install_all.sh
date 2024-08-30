#!/bin/bash

set -e

helm install \
    --set "listeners.client.protocol=PLAINTEXT" \
    kafka \
    oci://registry-1.docker.io/bitnamicharts/kafka

kubectl apply -f ./setup.yaml

cargo run --bin kafka-splitting -- generate-crds | kubectl apply -f ./setup.yaml

kubectl apply -f ./config.yaml
