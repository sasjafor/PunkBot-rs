FROM rust:1-alpine AS build

WORKDIR /app
RUN apk add --no-cache build-base cmake pkgconf opus-dev
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM node:24-alpine
RUN apk add --no-cache ca-certificates ffmpeg opus python3 py3-pip
COPY .release-versions/youtube-dlp-latest.txt /tmp/youtube-dlp-version
RUN yt_dlp_version="$(cat /tmp/youtube-dlp-version)" \
    && python3 -m pip install --break-system-packages --no-cache-dir "yt-dlp==${yt_dlp_version}" \
    && rm /tmp/youtube-dlp-version
WORKDIR /app
COPY --from=build /app/target/release/punk_bot /usr/local/bin/punk_bot
VOLUME ["/config"]
CMD ["/usr/local/bin/punk_bot"]
