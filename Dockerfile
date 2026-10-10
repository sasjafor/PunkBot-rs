FROM rust:1-bookworm AS build

WORKDIR /app
RUN apt-get update && apt-get install -y --no-install-recommends build-essential cmake pkg-config libopus-dev && rm -rf /var/lib/apt/lists/*
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM node:24-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg python3 libopus0 && rm -rf /var/lib/apt/lists/*
COPY .release-versions/youtube-dlp-latest.txt /tmp/youtube-dlp-version
RUN python3 -c 'from pathlib import Path; from urllib.request import urlretrieve; version = Path("/tmp/youtube-dlp-version").read_text().strip(); urlretrieve(f"https://github.com/yt-dlp/yt-dlp/releases/download/{version}/yt-dlp", "/usr/local/bin/yt-dlp")' \
    && chmod a+rx /usr/local/bin/yt-dlp \
    && rm /tmp/youtube-dlp-version
WORKDIR /app
COPY --from=build /app/target/release/punk_bot /usr/local/bin/punk_bot
VOLUME ["/config"]
CMD ["/usr/local/bin/punk_bot"]
