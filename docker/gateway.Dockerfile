FROM rust:1.85-slim AS build
WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config build-essential cmake protobuf-compiler ca-certificates git \
 && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --release -p gpucluster-gateway

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/gpucluster-gateway /usr/local/bin/gpucluster-gateway
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/gpucluster-gateway"]
