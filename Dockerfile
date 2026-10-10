FROM rust:1-bookworm AS build

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake pkg-config libopus-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg yt-dlp nodejs libopus0 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /app/target/release/punk_bot /usr/local/bin/punk_bot
VOLUME ["/config"]
CMD ["/usr/local/bin/punk_bot"]
