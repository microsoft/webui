# Copyright (c) Microsoft Corporation.
# Licensed under the MIT license.

FROM golang:1.25.0-bookworm@sha256:81dc45d05a7444ead8c92a389621fafabc8e40f8fd1a19d7e5df14e61e98bc1a AS go
FROM ubuntu:22.04@sha256:b000e69a50ff940390233025e7a337fb8469b509e2334fa4d018ac30f3e18cdb

RUN apt-get update -q \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends \
        build-essential binutils ca-certificates curl git protobuf-compiler \
        gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

COPY --from=go /usr/local/go /opt/go
ENV PATH="/opt/go/bin:${PATH}"
