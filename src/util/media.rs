use poise::serenity_prelude::async_trait;
use songbird::input::core::io::MediaSource;
use songbird::input::{AudioStream, AudioStreamError, Compose, Input, LiveInput, RawAdapter};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Mutex, mpsc},
    thread,
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
                    .is_file()
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

    pub async fn validate_filters(&self, filters: &str) -> Result<(), String> {
        if filters.trim().is_empty() {
            return Ok(());
        }
        let output = AsyncCommand::new("ffmpeg")
            .args([
                "-hide_banner",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-t",
                "0.1",
                "-af",
                filters,
                "-f",
                "null",
                "-",
            ])
            .output()
            .await
            .map_err(|error| format!("Could not validate FFmpeg filters: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let details = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if details.is_empty() {
            "FFmpeg rejected the filter expression".to_owned()
        } else {
            format!("FFmpeg rejected the filter expression: {details}")
        })
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
                "pcm_f32le",
                "-f",
                "f32le",
                "pipe:1",
            ])
            .stdin(Stdio::from(input))
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut transcoder = match transcoder.spawn() {
            Ok(child) => child,
            Err(error) => {
                let _ = downloader.kill();
                let _ = downloader.wait();
                return Err(AudioStreamError::Fail(Box::new(error)));
            }
        };
        let output = transcoder.stdout.take().ok_or_else(|| {
            AudioStreamError::Fail(Box::new(io::Error::other(
                "FFmpeg did not provide an output stream",
            )))
        })?;
        let source = match MediaPipeline::new(output, vec![downloader, transcoder]) {
            Ok(source) => source,
            Err((error, mut children)) => {
                for child in &mut children {
                    let _ = child.kill();
                }
                for child in children.iter_mut().rev() {
                    let _ = child.wait();
                }
                return Err(AudioStreamError::Fail(Box::new(error)));
            }
        };
        let input: Input = RawAdapter::new(source, 48_000, 2).into();
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

struct MediaPipeline {
    children: Vec<Child>,
    chunks: Mutex<mpsc::Receiver<Vec<u8>>>,
    current_chunk: Option<Vec<u8>>,
    current_offset: usize,
}

impl MediaPipeline {
    const READ_CHUNK_BYTES: usize = 32 * 1024;
    const BUFFERED_CHUNKS: usize = 24;

    fn new(
        mut output: std::process::ChildStdout,
        children: Vec<Child>,
    ) -> Result<Self, (io::Error, Vec<Child>)> {
        let (sender, receiver) = mpsc::sync_channel(Self::BUFFERED_CHUNKS);
        let worker = thread::Builder::new()
            .name("punkbot-audio-read-ahead".to_owned())
            .spawn(move || {
                loop {
                    let mut chunk = vec![0; Self::READ_CHUNK_BYTES];
                    match output.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(bytes_read) => {
                            chunk.truncate(bytes_read);
                            if sender.send(chunk).is_err() {
                                break;
                            }
                        }
                    }
                }
            });

        match worker {
            Ok(_) => Ok(Self {
                children,
                chunks: Mutex::new(receiver),
                current_chunk: None,
                current_offset: 0,
            }),
            Err(error) => Err((error, children)),
        }
    }
}

impl Read for MediaPipeline {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        if self.current_chunk.is_none() {
            let receiver = self
                .chunks
                .get_mut()
                .map_err(|_| io::Error::other("audio read-ahead buffer was poisoned"))?;
            self.current_chunk = receiver.recv().ok();
            self.current_offset = 0;
        }

        let Some(chunk) = self.current_chunk.as_ref() else {
            return Ok(0);
        };
        let bytes_read = buffer.len().min(chunk.len() - self.current_offset);
        buffer[..bytes_read]
            .copy_from_slice(&chunk[self.current_offset..self.current_offset + bytes_read]);
        self.current_offset += bytes_read;
        if self.current_offset == chunk.len() {
            self.current_chunk = None;
        }
        Ok(bytes_read)
    }
}

impl io::Seek for MediaPipeline {
    fn seek(&mut self, _: io::SeekFrom) -> io::Result<u64> {
        Err(io::Error::other("media pipeline does not support seeking"))
    }
}

impl MediaSource for MediaPipeline {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl Drop for MediaPipeline {
    fn drop(&mut self) {
        let upstream_count = self.children.len().saturating_sub(1);
        for child in self.children.iter_mut().take(upstream_count) {
            let _ = child.kill();
        }
        if let Some(transcoder) = self.children.last_mut() {
            let _ = transcoder.kill();
        }
        for child in self.children.iter_mut().rev() {
            let _ = child.wait();
        }
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
