# Multi-stage CUDA build for the cluster worker.
# Pick the CUDA tag matching your driver baseline:
#   12.8 → driver 555+ (Blackwell ready, RTX 5060 Ti)
#   12.4 → driver 535+
#   11.8 → driver 520+ (legacy)
ARG CUDA_VERSION=12.8.0
ARG UBUNTU_VERSION=24.04

FROM nvidia/cuda:${CUDA_VERSION}-cudnn-devel-ubuntu${UBUNTU_VERSION} AS build
WORKDIR /src

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl ca-certificates pkg-config build-essential cmake git \
    libssl-dev protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/opt/rust CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.82.0 --profile minimal

COPY . .
RUN cargo build --release -p gpucluster-worker

# Optional: build the C++ RPC extension when CUDA is present
RUN cmake -S cpp/llama-rpc-ext -B cpp/llama-rpc-ext/build \
      -DBUILD_RPC_SERVER=ON \
 && cmake --build cpp/llama-rpc-ext/build -j

FROM nvidia/cuda:${CUDA_VERSION}-cudnn-runtime-ubuntu${UBUNTU_VERSION}
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates wireguard-tools iproute2 iputils-ping \
 && rm -rf /var/lib/apt/lists/*

COPY --from=build /src/target/release/gpucluster-worker     /usr/local/bin/gpucluster-worker
COPY --from=build /src/cpp/llama-rpc-ext/build/rpc-server-ext /usr/local/bin/rpc-server-ext

ENV NODE_DATA_DIR=/var/lib/gpucluster
VOLUME ["/var/lib/gpucluster"]

ENTRYPOINT ["/usr/local/bin/gpucluster-worker"]
