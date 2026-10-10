use reqwest::{Client, Url};
use serde::Deserialize;
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub url: String,
    pub channel: Option<String>,
    pub thumbnail: Option<String>,
    pub duration: Option<u64>,
    pub age_restricted: bool,
}

#[derive(Debug, Clone)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub url: String,
    pub item_count: usize,
    pub channel: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Clone)]
pub struct YoutubeApi {
    client: Client,
    key: String,
    cache: Arc<Mutex<VideoCache>>,
}

#[derive(Default)]
struct VideoCache {
    entries: HashMap<String, Video>,
    order: VecDeque<String>,
}

impl VideoCache {
    fn get(&mut self, id: &str) -> Option<Video> {
        let video = self.entries.get(id)?.clone();
        self.order.retain(|key| key != id);
        self.order.push_back(id.to_owned());
        Some(video)
    }

    fn insert(&mut self, id: String, video: Video) {
        self.order.retain(|key| key != &id);
        self.order.push_back(id.clone());
        self.entries.insert(id, video);
        while self.entries.len() > 100 {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

#[derive(Deserialize, Default)]
struct Page<T> {
    #[serde(default)]
    items: Vec<T>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize, Default)]
struct Snippet {
    #[serde(default)]
    title: String,
    #[serde(rename = "channelTitle")]
    channel_title: Option<String>,
    thumbnails: Option<Thumbnails>,
    #[serde(rename = "resourceId")]
    resource_id: Option<ResourceId>,
}

#[derive(Deserialize, Default)]
struct Thumbnails {
    maxres: Option<Thumbnail>,
    standard: Option<Thumbnail>,
    high: Option<Thumbnail>,
    medium: Option<Thumbnail>,
    default: Option<Thumbnail>,
}

#[derive(Deserialize)]
struct Thumbnail {
    url: String,
}

impl Thumbnails {
    fn best(&self) -> Option<String> {
        self.maxres
            .as_ref()
            .or(self.standard.as_ref())
            .or(self.high.as_ref())
            .or(self.medium.as_ref())
            .or(self.default.as_ref())
            .map(|t| t.url.clone())
    }
}

#[derive(Deserialize, Default)]
struct ResourceId {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct SearchItem {
    id: SearchId,
    snippet: Snippet,
}

#[derive(Deserialize, Default)]
struct SearchId {
    #[serde(rename = "videoId")]
    video_id: Option<String>,
}

#[derive(Deserialize, Default)]
struct VideoItem {
    id: String,
    snippet: Snippet,
    #[serde(rename = "contentDetails")]
    content_details: ContentDetails,
}

#[derive(Deserialize, Default)]
struct ContentDetails {
    duration: Option<String>,
    #[serde(rename = "contentRating")]
    content_rating: Option<ContentRating>,
}

#[derive(Deserialize, Default)]
struct ContentRating {
    #[serde(rename = "ytRating")]
    yt_rating: Option<String>,
}

#[derive(Deserialize, Default)]
struct PlaylistItem {
    snippet: Snippet,
}

#[derive(Deserialize, Default)]
struct PlaylistInfoItem {
    id: String,
    snippet: Snippet,
    #[serde(rename = "contentDetails")]
    content_details: PlaylistDetails,
}

#[derive(Deserialize, Default)]
struct PlaylistDetails {
    #[serde(rename = "itemCount", default)]
    item_count: usize,
}

impl YoutubeApi {
    pub fn new(key: String) -> Self {
        Self {
            client: Client::new(),
            key,
            cache: Arc::new(Mutex::new(VideoCache::default())),
        }
    }

    async fn get<T: for<'de> Deserialize<'de>>(
        &self,
        endpoint: &str,
        pairs: &[(&str, &str)],
    ) -> Result<T, String> {
        let mut url = Url::parse(&format!(
            "https://youtube.googleapis.com/youtube/v3/{endpoint}"
        ))
        .map_err(|e| e.to_string())?;
        url.query_pairs_mut().append_pair("key", &self.key);
        for (key, value) in pairs {
            url.query_pairs_mut().append_pair(key, value);
        }
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "YouTube API returned {status}: {}",
                response.text().await.unwrap_or_default()
            ));
        }
        response.json().await.map_err(|e| e.to_string())
    }

    pub async fn search(&self, query: &str, count: usize) -> Result<Vec<Video>, String> {
        let limit = count.clamp(1, 50).to_string();
        let page: Page<SearchItem> = self
            .get(
                "search",
                &[
                    ("part", "snippet"),
                    ("type", "video"),
                    ("q", query),
                    ("maxResults", &limit),
                ],
            )
            .await?;
        Ok(page
            .items
            .into_iter()
            .filter_map(|item| {
                let id = item.id.video_id?;
                Some(Video {
                    url: format!("https://www.youtube.com/watch?v={id}"),
                    id,
                    title: item.snippet.title,
                    channel: item.snippet.channel_title,
                    thumbnail: item.snippet.thumbnails.and_then(|t| t.best()),
                    duration: None,
                    age_restricted: false,
                })
            })
            .collect())
    }

    pub async fn video(&self, id: &str) -> Result<Video, String> {
        if let Some(video) = self.cache.lock().await.get(id) {
            return Ok(video);
        }
        let page: Page<VideoItem> = self
            .get("videos", &[("part", "snippet,contentDetails"), ("id", id)])
            .await?;
        let video =
            page.items.into_iter().next().ok_or_else(|| {
                "Video is unavailable or not visible to the YouTube API".to_owned()
            })?;
        let video = Video {
            id: video.id.clone(),
            url: format!("https://www.youtube.com/watch?v={}", video.id),
            title: video.snippet.title,
            channel: video.snippet.channel_title,
            thumbnail: video.snippet.thumbnails.and_then(|t| t.best()),
            duration: video
                .content_details
                .duration
                .as_deref()
                .and_then(crate::util::parse_iso_duration),
            age_restricted: video
                .content_details
                .content_rating
                .and_then(|r| r.yt_rating)
                .as_deref()
                == Some("ytAgeRestricted"),
        };
        self.cache.lock().await.insert(id.to_owned(), video.clone());
        Ok(video)
    }

    pub async fn playlist(&self, id: &str) -> Result<Playlist, String> {
        let page: Page<PlaylistInfoItem> = self
            .get(
                "playlists",
                &[("part", "snippet,contentDetails"), ("id", id)],
            )
            .await?;
        let item = page
            .items
            .into_iter()
            .next()
            .ok_or_else(|| "Playlist is unavailable".to_owned())?;
        Ok(Playlist {
            id: item.id.clone(),
            title: item.snippet.title,
            url: format!("https://www.youtube.com/playlist?list={}", item.id),
            item_count: item.content_details.item_count,
            channel: item.snippet.channel_title,
            thumbnail: item.snippet.thumbnails.and_then(|t| t.best()),
        })
    }

    pub async fn playlist_video_ids(&self, id: &str) -> Result<Vec<String>, String> {
        let mut token: Option<String> = None;
        let mut ids = Vec::new();
        loop {
            let mut pairs = vec![
                ("part", "snippet"),
                ("playlistId", id),
                ("maxResults", "50"),
            ];
            if let Some(ref page_token) = token {
                pairs.push(("pageToken", page_token));
            }
            let page: Page<PlaylistItem> = self.get("playlistItems", &pairs).await?;
            ids.extend(
                page.items
                    .into_iter()
                    .filter_map(|item| item.snippet.resource_id.and_then(|r| r.video_id)),
            );
            token = page.next_page_token;
            if token.is_none() || ids.len() >= 5_000 {
                break;
            }
        }
        Ok(ids)
    }

    pub async fn video_url(&self, url: &str) -> Result<Video, String> {
        let id = youtube_video_id(url).ok_or_else(|| "Invalid YouTube video URL".to_owned())?;
        self.video(&id).await
    }
}

pub fn youtube_video_id(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?.trim_start_matches("www.");
    if host == "youtu.be" {
        return parsed
            .path_segments()?
            .next()
            .filter(|id| id.len() == 11)
            .map(str::to_owned);
    }
    if host == "youtube.com" || host.ends_with(".youtube.com") {
        if parsed.path() == "/watch" {
            return parsed
                .query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.into_owned());
        }
        let mut segments = parsed.path_segments()?;
        let first = segments.next()?;
        if ["embed", "v", "shorts"].contains(&first) {
            return segments.next().map(str::to_owned);
        }
    }
    None
}

pub fn youtube_time_offset(url: &str) -> u64 {
    let Some(parsed) = Url::parse(url).ok() else {
        return 0;
    };
    let Some(value) = parsed
        .query_pairs()
        .find(|(key, _)| key == "t")
        .map(|(_, value)| value.into_owned())
    else {
        return 0;
    };
    if let Ok(seconds) = value.parse::<u64>() {
        return seconds;
    }
    let mut number = String::new();
    let mut total = 0_u64;
    for character in value.chars() {
        if character.is_ascii_digit() {
            number.push(character);
            continue;
        }
        let Some(n) = number.parse::<u64>().ok() else {
            return 0;
        };
        match character {
            'h' => total += n * 3_600,
            'm' => total += n * 60,
            's' => total += n,
            _ => return 0,
        }
        number.clear();
    }
    if !number.is_empty() {
        total += number.parse::<u64>().unwrap_or(0);
    }
    total
}

pub fn youtube_playlist_id(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == "list")
        .map(|(_, v)| v.into_owned())
}
