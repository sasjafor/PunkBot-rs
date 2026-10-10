pub(super) use crate::{
    PContext, PError,
    util::{
        format_duration, parse_time,
        player::{Player, QueuedVideo},
        youtube::{Video, youtube_playlist_id, youtube_time_offset, youtube_video_id},
    },
};
pub(super) use poise::{CreateReply, futures_util::StreamExt, serenity_prelude as serenity};
pub(super) use serenity::{
    GuildId,
    all::{
        ButtonStyle, ComponentInteractionDataKind, CreateActionRow, CreateButton,
        CreateInteractionResponse, CreateInteractionResponseMessage, CreateSelectMenu,
        CreateSelectMenuKind, CreateSelectMenuOption,
    },
    model::id::ChannelId,
};
pub(super) use std::{path::Path, sync::Arc, time::Duration};
pub(super) use tokio::sync::Mutex;
pub(super) use tracing::warn;

pub(super) fn command_error(message: String) -> PError {
    std::io::Error::other(message).into()
}

pub(super) async fn reply(
    ctx: PContext<'_>,
    content: impl Into<String>,
    ephemeral: bool,
) -> Result<(), PError> {
    ctx.send(CreateReply::default().content(content).ephemeral(ephemeral))
        .await?;
    Ok(())
}

pub(super) fn guild_id(ctx: &PContext<'_>) -> Result<GuildId, PError> {
    ctx.guild_id()
        .ok_or_else(|| command_error("This command only works in a server".into()))
}

pub(super) fn author_channel(ctx: &PContext<'_>) -> Option<ChannelId> {
    let guild_id = ctx.guild_id()?;
    let guild = ctx.serenity_context().cache.guild(guild_id)?;
    guild
        .voice_states
        .get(&ctx.author().id)
        .and_then(|state| state.channel_id)
}

pub(super) async fn state(ctx: &PContext<'_>) -> Result<Arc<Mutex<Player>>, PError> {
    Ok(ctx.data().player_state(guild_id(ctx)?).await)
}

pub(super) fn queued(ctx: &PContext<'_>, video: Video, seek_seconds: u64) -> QueuedVideo {
    QueuedVideo {
        video,
        track_uuid: None,
        requester_id: ctx.author().id.get(),
        requester_name: ctx.author().name.clone(),
        requester_avatar: ctx.author().face(),
        seek_seconds,
    }
}

pub(super) async fn playback_embed(
    item: &QueuedVideo,
    queue_position: usize,
    player: &Player,
) -> serenity::CreateEmbed {
    let is_queued = queue_position > 0;
    let duration = item
        .video
        .duration
        .map(format_duration)
        .unwrap_or_else(|| "Unknown".to_owned());
    let mut embed = serenity::CreateEmbed::new()
        .title(item.video.title.clone())
        .url(item.video.url.clone())
        .author(
            serenity::CreateEmbedAuthor::new(if is_queued {
                "Added to queue"
            } else {
                "Playing"
            })
            .icon_url(item.requester_avatar.clone())
            .url("https://github.com/sasjafor/PunkBot"),
        )
        .colour(serenity::Colour::from_rgb(0, 86, 191))
        .field("Song Duration", duration, true)
        .field("Requested by", format!("<@{}>", item.requester_id), true);
    if let Some(channel) = item.video.channel.as_deref() {
        embed = embed.field("Channel", channel, true);
    }
    if let Some(thumbnail) = item.video.thumbnail.as_deref() {
        embed = embed.thumbnail(thumbnail);
    }
    if is_queued {
        let estimated_wait = if player.looping {
            "∞".to_owned()
        } else {
            player
                .estimated_wait(queue_position)
                .await
                .map(format_duration)
                .unwrap_or_else(|| "Unknown".to_owned())
        };
        embed = embed
            .field("Estimated time until playing", estimated_wait, true)
            .field("Position in queue", queue_position.to_string(), true);
    }
    embed
}

pub(super) async fn play_result(
    ctx: PContext<'_>,
    query: String,
    next: bool,
    silent: bool,
) -> Result<(), PError> {
    if silent {
        ctx.defer_ephemeral().await?;
    } else {
        ctx.defer().await?;
    }
    let channel_id =
        author_channel(&ctx).ok_or_else(|| command_error("Join a voice channel first".into()))?;
    let data = ctx.data();
    let mut videos = Vec::new();
    let mut playlist_info = None;
    let mut initial_seek_seconds = 0;

    if let Some(id) = youtube_playlist_id(&query) {
        let playlist = data.youtube.playlist(&id).await.map_err(command_error)?;
        playlist_info = Some(playlist);
        let ids = data
            .youtube
            .playlist_video_ids(&id)
            .await
            .map_err(command_error)?;
        for video_id in ids {
            match data.youtube.video(&video_id).await {
                Ok(video) => videos.push(video),
                Err(error) => warn!("Skipping unavailable playlist entry {video_id}: {error}"),
            }
        }
    } else if youtube_video_id(&query).is_some() {
        initial_seek_seconds = youtube_time_offset(&query);
        videos.push(
            data.youtube
                .video_url(&query)
                .await
                .map_err(command_error)?,
        );
    } else if query.starts_with("http://") || query.starts_with("https://") {
        let title = Path::new(query.split('?').next().unwrap_or(&query))
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Audio stream")
            .to_owned();
        let duration = data.media.duration(&query).await;
        videos.push(Video {
            id: query.clone(),
            title,
            url: query.clone(),
            channel: None,
            thumbnail: None,
            duration,
            age_restricted: false,
        });
    } else {
        let results = data
            .youtube
            .search(&query, 1)
            .await
            .map_err(command_error)?;
        let found = results
            .into_iter()
            .next()
            .ok_or_else(|| command_error("No matches".into()))?;
        videos.push(data.youtube.video(&found.id).await.map_err(command_error)?);
    }

    if videos.is_empty() {
        return Err(command_error("No playable videos were found".into()));
    }
    if videos.iter().any(|video| video.age_restricted) && data.media.cookies.is_none() {
        return Err(command_error(
            "This video is age restricted. Configure YOUTUBE_COOKIES_FILE to play it.".into(),
        ));
    }

    let player = data.player_state(guild_id(&ctx)?).await;
    let total = videos.len();
    let requested_video = videos.first().cloned();
    let seek_video_id = videos.first().map(|video| video.id.clone());
    let mut queued_position = 0;
    // Preserve playlist order when placing several tracks at the front.
    if next {
        videos.reverse();
    }
    for video in videos {
        let seek_seconds = if seek_video_id.as_deref() == Some(video.id.as_str()) {
            initial_seek_seconds
        } else {
            0
        };
        queued_position = Player::enqueue(
            &ctx,
            player.clone(),
            queued(&ctx, video, seek_seconds),
            channel_id,
            next,
        )
        .await
        .map_err(command_error)?;
    }
    let embed = if let Some(playlist) = playlist_info {
        let mut embed = serenity::CreateEmbed::new()
            .title(playlist.title)
            .url(playlist.url)
            .author(
                serenity::CreateEmbedAuthor::new("Enqueued playlist")
                    .icon_url(ctx.author().face())
                    .url("https://github.com/sasjafor/PunkBot"),
            )
            .field(
                "Enqueued Items",
                format!("{total}/{}", playlist.item_count),
                true,
            );
        if let Some(channel) = playlist.channel {
            embed = embed.field("Channel", channel, true);
        }
        if let Some(thumbnail) = playlist.thumbnail {
            embed = embed.thumbnail(thumbnail);
        }
        embed
    } else {
        let item = queued(
            &ctx,
            requested_video.expect("playable results were checked above"),
            initial_seek_seconds,
        );
        let state = player.lock().await;
        playback_embed(&item, queued_position, &state).await
    };
    ctx.send(CreateReply::default().embed(embed).ephemeral(silent))
        .await?;
    Ok(())
}
