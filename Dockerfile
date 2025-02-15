FROM rust:1.79 as builder

RUN apt-get update && apt-get install -y \
    build-essential \
    pkg-config \
    libssl-dev \
    curl \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install latest protoc
RUN PROTOC_VERSION=$(curl -s https://api.github.com/repos/protocolbuffers/protobuf/releases/latest | grep -oP '"tag_name": "\K(.*)(?=")') && \
    PROTOC_VERSION=${PROTOC_VERSION#v} && \
    curl -LO "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-x86_64.zip" && \
    unzip "protoc-${PROTOC_VERSION}-linux-x86_64.zip" -d /usr/local && \
    rm "protoc-${PROTOC_VERSION}-linux-x86_64.zip"

# Copy manifests and build only the dependencies to cache them
RUN USER=root cargo new --bin pump-rs
WORKDIR /pump-rs
COPY ./Cargo.toml ./Cargo.toml
COPY ./Cargo.lock ./Cargo.lock

RUN cargo update
RUN cargo build --release
RUN rm src/*.rs

# Copy over source
COPY ./src ./src

# Build for release
RUN cargo build --release

FROM ubuntu:22.04 as runner

RUN apt-get update && apt-get install -y \
    ca-certificates \
    openssl \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /pump-rs/target/release/pump-rs .

CMD ["./pump-rs", "sanity"]
