use poise::serenity_prelude::async_trait;
use songbird::input::core::io::MediaSource;
use songbird::input::{AudioStream, AudioStreamError, ChildContainer, Compose, Input, LiveInput};
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tokio::process::Command as AsyncCommand;

#[derive(Clone, Default)]
pub struct MediaSettings {
    pub cookies: Option<PathBuf>,
}

impl MediaSettings {
    pub fn from_environment() -> Self {
        let cookies = std::env::var_os("YOUTUBE_COOKIES_FILE")
            .map(PathBuf::from)
            .filter(|path| path.is_file())
            .or_else(|| {
                Path::new(".data/youtube.data")
                    .exists()
                    .then(|| PathBuf::from(".data/youtube.data"))
            });
        Self { cookies }
    }

    pub fn input(&self, url: &str, seek_seconds: u64, filters: &str) -> Result<Input, String> {
        Ok(Input::Lazy(Box::new(MediaInput {
            url: url.to_owned(),
            seek_seconds,
            filters: filters.to_owned(),
            cookies: self.cookies.clone(),
        })))
    }

    fn spawn(
        &self,
        url: &str,
        seek_seconds: u64,
        filters: &str,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        let mut downloader = Command::new("yt-dlp");
        downloader.args([
            "--no-playlist",
            "--no-part",
            "--js-runtimes",
            "node",
            "-f",
            "bestaudio/best",
            "-o",
            "-",
        ]);
        if let Some(cookies) = &self.cookies {
            downloader.arg("--cookies").arg(cookies);
        }
        downloader
            .arg("--")
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut downloader = downloader
            .spawn()
            .map_err(|e| AudioStreamError::Fail(Box::new(e)))?;
        let input = downloader.stdout.take().ok_or_else(|| {
            AudioStreamError::Fail(Box::new(std::io::Error::other(
                "yt-dlp did not provide an output stream",
            )))
        })?;

        let mut transcoder = Command::new("ffmpeg");
        transcoder.args(["-hide_banner", "-loglevel", "error"]);
        transcoder.args(["-i", "pipe:0"]);
        if seek_seconds > 0 {
            transcoder.args(["-ss", &seek_seconds.to_string()]);
        }
        if !filters.trim().is_empty() {
            transcoder.args(["-af", filters]);
        }
        transcoder
            .args([
                "-vn",
                "-ac",
                "2",
                "-ar",
                "48000",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
                "pipe:1",
            ])
            .stdin(Stdio::from(input))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let transcoder = match transcoder.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = downloader.kill();
                let _ = downloader.wait();
                return Err(AudioStreamError::Fail(Box::new(error)));
            }
        };
        let input: Input = ChildContainer::new(vec![downloader, transcoder]).into();
        match input {
            Input::Live(LiveInput::Raw(stream), _) => Ok(stream),
            _ => Err(AudioStreamError::Fail(Box::new(std::io::Error::other(
                "unexpected media input state",
            )))),
        }
    }

    pub async fn duration(&self, url: &str) -> Option<u64> {
        let output = AsyncCommand::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(url)
            .output()
            .await
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| v.max(0.0) as u64)
    }
}

#[derive(Debug)]
struct MediaInput {
    url: String,
    seek_seconds: u64,
    filters: String,
    cookies: Option<PathBuf>,
}

#[async_trait]
impl Compose for MediaInput {
    fn create(&mut self) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        Err(AudioStreamError::Unsupported)
    }

    async fn create_async(
        &mut self,
    ) -> Result<AudioStream<Box<dyn MediaSource>>, AudioStreamError> {
        let settings = MediaSettings {
            cookies: self.cookies.clone(),
        };
        settings.spawn(&self.url, self.seek_seconds, &self.filters)
    }

    fn should_create_async(&self) -> bool {
        true
    }
}
