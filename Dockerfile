FROM rust:1-bookworm AS build

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake pkg-config libopus-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM node:24-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg python3 libopus0 && rm -rf /var/lib/apt/lists/*
ADD https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp /usr/local/bin/yt-dlp
RUN chmod a+rx /usr/local/bin/yt-dlp
WORKDIR /app
COPY --from=build /app/target/release/punk_bot /usr/local/bin/punk_bot
VOLUME ["/config"]
CMD ["/usr/local/bin/punk_bot"]
