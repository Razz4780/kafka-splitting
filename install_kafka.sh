#!/bin/bash

helm install \
    --set "listeners.client.protocol=PLAINTEXT" \
    kafka \
    oci://registry-1.docker.io/bitnamicharts/kafka
