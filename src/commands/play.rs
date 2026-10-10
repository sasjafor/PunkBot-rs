use super::common::*;
use crate::{PContext, PError};

#[doc = "Play a video, playlist, search result, or supported media URL."]
#[poise::command(slash_command, guild_only)]
pub async fn play(
    ctx: PContext<'_>,
    #[description = "YouTube link or search term"] query: String,
    #[description = "Reply only to you"] silent: Option<bool>,
    #[description = "Place the track next"] next: Option<bool>,
) -> Result<(), PError> {
    play_result(ctx, query, next.unwrap_or(false), silent.unwrap_or(false)).await
}
