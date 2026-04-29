FROM rust:1.85-slim AS build
WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config build-essential cmake protobuf-compiler ca-certificates git \
 && rm -rf /var/lib/apt/lists/*

COPY . .
RUN cargo build --release -p gpucluster-mgmt-backend

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/gpucluster-mgmt /usr/local/bin/gpucluster-mgmt
COPY --from=build /src/crates/mgmt-backend/migrations /opt/gpucluster/migrations
EXPOSE 7100
ENTRYPOINT ["/usr/local/bin/gpucluster-mgmt"]
