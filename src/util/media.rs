use poise::serenity_prelude::async_trait;
use songbird::input::core::io::MediaSource;
use songbird::input::{AudioStream, AudioStreamError, Compose, Input, LiveInput, RawAdapter};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};
use tokio::process::Command as AsyncCommand;
use tokio::sync::Notify;

const DEFAULT_AUDIO_CACHE_MAX_BYTES: u64 = 1024 * 1024 * 1024;
const AUDIO_CACHE_DIRECTORY: &str = "punkbot-rs-audio-cache";
static NEXT_CACHE_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct MediaSettings {
    pub cookies: Option<PathBuf>,
    audio_cache: Arc<AudioCacheBudget>,
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
        let max_bytes = std::env::var("AUDIO_CACHE_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_AUDIO_CACHE_MAX_BYTES);
        Self {
            cookies,
            audio_cache: Arc::new(AudioCacheBudget::new(max_bytes)),
        }
    }

    pub fn input(&self, url: &str, seek_seconds: u64, filters: &str) -> Result<Input, String> {
        Ok(Input::Lazy(Box::new(MediaInput {
            url: url.to_owned(),
            seek_seconds,
            filters: filters.to_owned(),
            cookies: self.cookies.clone(),
            replay_cache: Arc::new(ReplayCache::new(self.audio_cache.clone())),
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
        replay_cache: Arc<ReplayCache>,
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
        replay_cache.start();
        let source = match MediaPipeline::new(
            output,
            vec![downloader, transcoder],
            Some(replay_cache.clone()),
        ) {
            Ok(source) => source,
            Err((error, mut children)) => {
                replay_cache.disable();
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

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            cookies: None,
            audio_cache: Arc::new(AudioCacheBudget::new(DEFAULT_AUDIO_CACHE_MAX_BYTES)),
        }
    }
}

struct AudioCacheBudget {
    max_bytes: u64,
    used_bytes: AtomicU64,
}

impl AudioCacheBudget {
    fn new(max_bytes: u64) -> Self {
        cleanup_orphaned_audio_caches();
        let _ = thread::Builder::new()
            .name("punkbot-audio-cache-cleanup".to_owned())
            .spawn(|| {
                loop {
                    thread::sleep(Duration::from_secs(10 * 60));
                    cleanup_orphaned_audio_caches();
                }
            });
        Self {
            max_bytes,
            used_bytes: AtomicU64::new(0),
        }
    }

    fn reserve(&self, bytes: u64) -> bool {
        let mut used = self.used_bytes.load(Ordering::Relaxed);
        loop {
            let Some(next) = used.checked_add(bytes) else {
                return false;
            };
            if next > self.max_bytes {
                return false;
            }
            match self.used_bytes.compare_exchange_weak(
                used,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(actual) => used = actual,
            }
        }
    }

    fn release(&self, bytes: u64) {
        self.used_bytes.fetch_sub(bytes, Ordering::AcqRel);
    }

    fn create_file(&self) -> io::Result<(PathBuf, File)> {
        let directory = std::env::temp_dir().join(AUDIO_CACHE_DIRECTORY);
        fs::create_dir_all(&directory)?;
        loop {
            let id = NEXT_CACHE_FILE_ID.fetch_add(1, Ordering::Relaxed);
            let path = directory.join(format!("{}-{id}.f32le", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => {
                    if let Err(error) = file.try_lock() {
                        let _ = fs::remove_file(&path);
                        return Err(match error {
                            std::fs::TryLockError::Error(error) => error,
                            std::fs::TryLockError::WouldBlock => {
                                io::Error::other("new audio cache file was unexpectedly locked")
                            }
                        });
                    }
                    return Ok((path, file));
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
    }
}

fn cleanup_orphaned_audio_caches() {
    let directory = std::env::temp_dir().join(AUDIO_CACHE_DIRECTORY);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_none_or(|extension| extension != "f32le")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let Some((pid, id)) = stem.split_once('-') else {
            continue;
        };
        if pid.parse::<u32>().is_err() || id.parse::<u64>().is_err() {
            continue;
        }
        let is_stale = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age >= Duration::from_secs(60));
        if !is_stale {
            continue;
        }
        let Ok(file) = OpenOptions::new().read(true).open(&path) else {
            continue;
        };
        if file.try_lock().is_ok() {
            let _ = fs::remove_file(path);
        }
    }
}

enum ReplayCacheState {
    Empty,
    Writing {
        path: PathBuf,
        file: File,
        bytes: u64,
    },
    Ready {
        path: PathBuf,
        bytes: u64,
        _lease: File,
    },
    Disabled,
}

struct ReplayCache {
    budget: Arc<AudioCacheBudget>,
    state: Mutex<ReplayCacheState>,
    changed: Notify,
}

impl ReplayCache {
    fn new(budget: Arc<AudioCacheBudget>) -> Self {
        Self {
            budget,
            state: Mutex::new(ReplayCacheState::Empty),
            changed: Notify::new(),
        }
    }

    fn start(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !matches!(*state, ReplayCacheState::Empty) {
            return matches!(
                *state,
                ReplayCacheState::Writing { .. } | ReplayCacheState::Ready { .. }
            );
        }
        if self.budget.max_bytes == 0 {
            *state = ReplayCacheState::Disabled;
            self.changed.notify_one();
            return false;
        }
        match self.budget.create_file() {
            Ok((path, file)) => {
                *state = ReplayCacheState::Writing {
                    path,
                    file,
                    bytes: 0,
                };
                true
            }
            Err(_) => {
                *state = ReplayCacheState::Disabled;
                self.changed.notify_one();
                false
            }
        }
    }

    fn append(&self, bytes: &[u8]) {
        if bytes.is_empty() || !self.budget.reserve(bytes.len() as u64) {
            if !bytes.is_empty() {
                self.disable();
            }
            return;
        }

        let Ok(mut state) = self.state.lock() else {
            self.budget.release(bytes.len() as u64);
            self.disable();
            return;
        };
        let ReplayCacheState::Writing {
            path,
            file,
            bytes: written,
        } = &mut *state
        else {
            self.budget.release(bytes.len() as u64);
            return;
        };
        if file.write_all(bytes).is_err() {
            let path = path.clone();
            let written = *written + bytes.len() as u64;
            *state = ReplayCacheState::Disabled;
            self.budget.release(written);
            let _ = fs::remove_file(path);
            self.changed.notify_one();
            return;
        }
        *written += bytes.len() as u64;
    }

    fn finish(&self) {
        let Ok(mut state) = self.state.lock() else {
            self.disable();
            return;
        };
        let old = std::mem::replace(&mut *state, ReplayCacheState::Disabled);
        let ReplayCacheState::Writing {
            path,
            mut file,
            bytes,
        } = old
        else {
            *state = old;
            return;
        };
        if file.flush().is_ok() {
            *state = ReplayCacheState::Ready {
                path,
                bytes,
                _lease: file,
            };
        } else {
            drop(file);
            self.budget.release(bytes);
            let _ = fs::remove_file(path);
        }
        self.changed.notify_one();
    }

    fn disable(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let old = std::mem::replace(&mut *state, ReplayCacheState::Disabled);
        match old {
            ReplayCacheState::Writing { path, file, bytes } => {
                drop(file);
                self.budget.release(bytes);
                let _ = fs::remove_file(path);
            }
            ReplayCacheState::Ready {
                path,
                bytes,
                _lease,
            } => {
                drop(_lease);
                self.budget.release(bytes);
                let _ = fs::remove_file(path);
            }
            ReplayCacheState::Empty | ReplayCacheState::Disabled => {}
        }
        self.changed.notify_one();
    }

    async fn completed_path(&self) -> Option<PathBuf> {
        loop {
            let notified = self.changed.notified();
            {
                let state = self.state.lock().ok()?;
                match &*state {
                    ReplayCacheState::Ready { path, .. } => return Some(path.clone()),
                    ReplayCacheState::Writing { .. } => {}
                    ReplayCacheState::Empty | ReplayCacheState::Disabled => return None,
                }
            }
            notified.await;
        }
    }
}

impl Drop for ReplayCache {
    fn drop(&mut self) {
        let Ok(state) = self.state.get_mut() else {
            return;
        };
        match std::mem::replace(state, ReplayCacheState::Disabled) {
            ReplayCacheState::Writing { path, file, bytes } => {
                drop(file);
                self.budget.release(bytes);
                let _ = fs::remove_file(path);
            }
            ReplayCacheState::Ready {
                path,
                bytes,
                _lease,
            } => {
                drop(_lease);
                self.budget.release(bytes);
                let _ = fs::remove_file(path);
            }
            ReplayCacheState::Empty | ReplayCacheState::Disabled => {}
        }
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
        replay_cache: Option<Arc<ReplayCache>>,
    ) -> Result<Self, (io::Error, Vec<Child>)> {
        let (sender, receiver) = mpsc::sync_channel(Self::BUFFERED_CHUNKS);
        let worker_cache = replay_cache.clone();
        let worker = thread::Builder::new()
            .name("punkbot-audio-read-ahead".to_owned())
            .spawn(move || {
                loop {
                    let mut chunk = vec![0; Self::READ_CHUNK_BYTES];
                    match output.read(&mut chunk) {
                        Ok(0) => {
                            if let Some(cache) = &worker_cache {
                                cache.finish();
                            }
                            break;
                        }
                        Err(_) => {
                            if let Some(cache) = &worker_cache {
                                cache.disable();
                            }
                            break;
                        }
                        Ok(bytes_read) => {
                            chunk.truncate(bytes_read);
                            if let Some(cache) = &worker_cache {
                                cache.append(&chunk);
                            }
                            if sender.send(chunk).is_err() {
                                if let Some(cache) = &worker_cache {
                                    cache.disable();
                                }
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

struct MediaInput {
    url: String,
    seek_seconds: u64,
    filters: String,
    cookies: Option<PathBuf>,
    replay_cache: Arc<ReplayCache>,
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
            audio_cache: self.replay_cache.budget.clone(),
        };
        if let Some(path) = self.replay_cache.completed_path().await {
            match File::open(path) {
                Ok(file) => {
                    let input: Input = RawAdapter::new(file, 48_000, 2).into();
                    match input {
                        Input::Live(LiveInput::Raw(stream), _) => Ok(stream),
                        _ => Err(AudioStreamError::Fail(Box::new(io::Error::other(
                            "unexpected cached media input state",
                        )))),
                    }
                }
                Err(_) => {
                    self.replay_cache.disable();
                    settings.spawn(
                        &self.url,
                        self.seek_seconds,
                        &self.filters,
                        self.replay_cache.clone(),
                    )
                }
            }
        } else {
            settings.spawn(
                &self.url,
                self.seek_seconds,
                &self.filters,
                self.replay_cache.clone(),
            )
        }
    }

    fn should_create_async(&self) -> bool {
        true
    }
}
