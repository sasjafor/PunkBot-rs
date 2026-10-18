use crate::{Data, util::youtube::Video};
use poise::serenity_prelude as serenity;
use serenity::GuildId;
use serenity::{async_trait, model::id::ChannelId};
use songbird::Songbird;
use songbird::{
    events::{Event, EventContext, EventHandler as VoiceEventHandler, TrackEvent},
    input::Input,
    tracks::{PlayMode, Track, TrackHandle, TrackQueue},
};
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone, Debug)]
pub struct QueuedVideo {
    pub video: Video,
    pub track_uuid: Option<u128>,
    pub requester_id: u64,
    pub requester_name: String,
    pub requester_avatar: String,
    pub seek_seconds: u64,
}

pub struct Player {
    pub channel_id: Option<ChannelId>,
    pub items: VecDeque<QueuedVideo>,
    pub looping: bool,
    pub volume: f32,
    pub filters: String,
    pub track: Option<TrackHandle>,
    pub generation: u64,
}

impl Default for Player {
    fn default() -> Self {
        Self {
            channel_id: None,
            items: VecDeque::new(),
            looping: false,
            volume: 1.0,
            filters: String::new(),
            track: None,
            generation: 0,
        }
    }
}

impl Player {
    pub async fn enqueue(
        ctx: &poise::Context<'_, Data, crate::PError>,
        state: Arc<Mutex<Self>>,
        mut video: QueuedVideo,
        channel_id: ChannelId,
        next: bool,
    ) -> Result<usize, String> {
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call_lock = manager
            .join(guild_id, channel_id)
            .await
            .map_err(|e| format!("Could not join voice channel: {e}"))?;
        let mut player = state.lock().await;
        let input =
            ctx.data()
                .media
                .input(&video.video.url, video.seek_seconds, &player.filters)?;
        let track = Track::from(input).volume(player.volume.max(0.0));
        let mut call = call_lock.lock().await;
        let queue_was_empty = call.queue().is_empty();
        let handle = call.enqueue(track).await;
        player.generation = player.generation.wrapping_add(1);
        video.track_uuid = Some(handle.uuid().as_u128());
        let queue = call.queue();
        if next && queue.len() > 1 {
            queue.modify_queue(|items| {
                if let Some(last) = items.pop_back() {
                    items.insert(1, last);
                }
            });
            let insert_at = 1.min(player.items.len());
            player.items.insert(insert_at, video);
        } else {
            player.items.push_back(video);
        }
        add_track_events(
            &handle,
            state.clone(),
            queue.clone(),
            manager.clone(),
            guild_id,
        )?;
        if player.looping && queue.len() == 1 {
            let _ = handle.enable_loop();
        }
        player.channel_id = Some(channel_id);
        if queue_was_empty {
            player.track = Some(handle);
        }
        Ok(if next {
            1
        } else {
            player.items.len().saturating_sub(1)
        })
    }

    pub async fn current_position(&self) -> u64 {
        let seek = self
            .items
            .front()
            .map(|item| item.seek_seconds)
            .unwrap_or(0);
        match &self.track {
            Some(track) => track
                .get_info()
                .await
                .map(|s| s.position.as_secs().saturating_add(seek))
                .unwrap_or(seek),
            None => seek,
        }
    }

    pub fn queue_len(&self) -> usize {
        self.items.len().saturating_sub(1)
    }

    pub async fn estimated_wait(&self, queue_position: usize) -> Option<u64> {
        if queue_position == 0 {
            return Some(0);
        }
        let current = self.items.front()?;
        let mut total = current
            .video
            .duration?
            .saturating_sub(self.current_position().await);
        for item in self
            .items
            .iter()
            .skip(1)
            .take(queue_position.saturating_sub(1))
        {
            total = total.checked_add(item.video.duration?)?;
        }
        Some(total)
    }

    pub async fn status(&self) -> Option<PlayMode> {
        match &self.track {
            Some(track) => track.get_info().await.ok().map(|s| s.playing),
            None => None,
        }
    }

    pub async fn set_loop(&mut self) -> bool {
        self.looping = !self.looping;
        if let Some(track) = &self.track {
            if self.looping {
                let _ = track.enable_loop();
            } else {
                let _ = track.disable_loop();
            }
        }
        self.looping
    }

    pub async fn clear_waiting(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
    ) -> Result<(), String> {
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        let queue = call.lock().await.queue().clone();
        queue.modify_queue(|tracks| {
            while tracks.len() > 1 {
                if let Some(track) = tracks.pop_back() {
                    let _ = track.stop();
                }
            }
        });
        self.items.truncate(1);
        Ok(())
    }

    pub async fn stop(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
    ) -> Result<(), String> {
        if self.items.is_empty() {
            return Err("Nothing is playing".into());
        }
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        call.lock().await.queue().stop();
        self.items.clear();
        self.track = None;
        self.generation = self.generation.wrapping_add(1);
        schedule_disconnect(
            ctx.data().voice.clone(),
            ctx.data().player_state(guild_id).await,
            guild_id,
            self.generation,
        );
        Ok(())
    }

    pub async fn disconnect(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
    ) -> Result<(), String> {
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        manager.remove(guild_id).await.map_err(|e| e.to_string())?;
        self.items.clear();
        self.track = None;
        self.channel_id = None;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub async fn seek(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
        seconds: u64,
    ) -> Result<(), String> {
        let Some(current) = self.items.front().cloned() else {
            return Err("Nothing is playing".into());
        };
        if current
            .video
            .duration
            .is_some_and(|duration| seconds > duration)
        {
            return Err("Time cannot be longer than the song".into());
        }
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call_lock = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        let mut call = call_lock.lock().await;
        let queue = call.queue().clone();
        self.items.front_mut().map(|item| item.track_uuid = None);
        if let Some(old) = queue.dequeue(0) {
            let _ = old.stop();
        }
        let input: Input = ctx
            .data()
            .media
            .input(&current.video.url, seconds, &self.filters)?;
        let handle = call
            .enqueue(Track::from(input).volume(self.volume.max(0.0)))
            .await;
        let queue = call.queue().clone();
        self.items
            .front_mut()
            .map(|item| item.track_uuid = Some(handle.uuid().as_u128()));
        queue.modify_queue(|tracks| {
            if let Some(last) = tracks.pop_back() {
                tracks.push_front(last);
            }
        });
        if let Some(current) = queue.current() {
            let _ = current.play();
        }
        let state = ctx.data().player_state(guild_id).await;
        add_track_events(&handle, state, queue.clone(), manager, guild_id)?;
        if self.looping {
            let _ = handle.enable_loop();
        }
        self.track = Some(handle);
        self.items
            .front_mut()
            .map(|item| item.seek_seconds = seconds);
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }

    pub async fn set_filters(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
        filters: String,
    ) -> Result<(), String> {
        ctx.data().media.validate_filters(&filters).await?;
        let previous_filters = std::mem::replace(&mut self.filters, filters);
        if self.items.is_empty() {
            return Ok(());
        }
        if let Err(error) = self.rebuild_queue(ctx, true).await {
            self.filters = previous_filters;
            return Err(error);
        }
        Ok(())
    }

    pub async fn set_volume(&mut self, ctx: &poise::Context<'_, Data, crate::PError>, value: f32) {
        self.volume = value;
        let Some(guild_id) = ctx.guild_id() else {
            return;
        };
        let Some(call) = ctx.data().voice.get(guild_id) else {
            return;
        };
        for track in call.lock().await.queue().current_queue() {
            let _ = track.set_volume(value);
        }
    }

    pub async fn remove_waiting(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
        index: usize,
    ) -> Result<QueuedVideo, String> {
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call_lock = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        let queue = call_lock.lock().await.queue().clone();
        let actual_index = index.saturating_add(1);
        let queued = queue
            .dequeue(actual_index)
            .ok_or("Queue position is out of range")?;
        let _ = queued.stop();
        self.items
            .remove(actual_index)
            .ok_or("Queue position is out of range".into())
    }

    pub async fn shuffle_waiting(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
    ) -> Result<(), String> {
        use rand::seq::SliceRandom;
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call_lock = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        let mut order = (0..self.items.len().saturating_sub(1)).collect::<Vec<_>>();
        order.shuffle(&mut rand::rng());
        call_lock.lock().await.queue().modify_queue(|tracks| {
            let current = tracks.pop_front();
            let mut waiting = tracks.drain(..).map(Some).collect::<Vec<_>>();
            if let Some(current) = current {
                tracks.push_back(current);
            }
            for index in &order {
                if let Some(Some(track)) = waiting.get_mut(*index).map(Option::take) {
                    tracks.push_back(track);
                }
            }
        });
        if !self.items.is_empty() {
            let current = self.items.pop_front();
            let mut waiting = std::mem::take(&mut self.items)
                .into_iter()
                .map(Some)
                .collect::<Vec<_>>();
            if let Some(current) = current {
                self.items.push_back(current);
            }
            for index in order {
                if let Some(item) = waiting.get_mut(index).and_then(Option::take) {
                    self.items.push_back(item);
                }
            }
        }
        Ok(())
    }

    pub async fn pause(&mut self) -> Result<(), String> {
        let track = self.track.as_ref().ok_or("Nothing is playing")?;
        let state = track.get_info().await.map_err(|e| e.to_string())?;
        if state.playing == PlayMode::Pause {
            return Err("Already paused".into());
        }
        track.pause().map_err(|e| e.to_string())
    }

    pub async fn resume(&mut self) -> Result<(), String> {
        let track = self.track.as_ref().ok_or("Nothing is playing")?;
        let state = track.get_info().await.map_err(|e| e.to_string())?;
        if state.playing == PlayMode::Play {
            return Err("Already playing".into());
        }
        track.play().map_err(|e| e.to_string())
    }

    pub async fn skip(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
    ) -> Result<(String, Option<String>), String> {
        let title = self
            .items
            .front()
            .map(|item| item.video.title.clone())
            .ok_or("Nothing is playing")?;
        let next_title = self.items.get(1).map(|item| item.video.title.clone());
        let manager = ctx.data().voice.clone();
        let guild_id = ctx
            .guild_id()
            .ok_or("This command only works in a server")?;
        let call = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        if self.looping {
            if let Some(track) = &self.track {
                let _ = track.disable_loop();
            }
        }
        call.lock()
            .await
            .queue()
            .skip()
            .map_err(|e| e.to_string())?;
        Ok((title, next_title))
    }

    async fn rebuild_queue(
        &mut self,
        ctx: &poise::Context<'_, Data, crate::PError>,
        seek_current: bool,
    ) -> Result<(), String> {
        let Some(guild_id) = ctx.guild_id() else {
            return Err("This command only works in a server".into());
        };
        let manager = ctx.data().voice.clone();
        let call_lock = manager
            .get(guild_id)
            .ok_or("I am not connected to a voice channel")?;
        let mut call = call_lock.lock().await;
        let queue = call.queue().clone();
        let current_position = if seek_current {
            self.current_position().await
        } else {
            self.items
                .front()
                .map(|item| item.seek_seconds)
                .unwrap_or(0)
        };
        let state = ctx.data().player_state(guild_id).await;
        for item in &mut self.items {
            item.track_uuid = None;
        }
        if seek_current {
            if let Some(item) = self.items.front_mut() {
                item.seek_seconds = current_position;
            }
        }
        queue.stop();
        let mut items = std::mem::take(&mut self.items);
        let mut first_track = None;
        for item in &mut items {
            let input =
                ctx.data()
                    .media
                    .input(&item.video.url, item.seek_seconds, &self.filters)?;
            let handle = call
                .enqueue(Track::from(input).volume(self.volume.max(0.0)))
                .await;
            item.track_uuid = Some(handle.uuid().as_u128());
            add_track_events(
                &handle,
                state.clone(),
                queue.clone(),
                manager.clone(),
                guild_id,
            )?;
            if first_track.is_none() {
                if self.looping {
                    let _ = handle.enable_loop();
                }
                first_track = Some(handle);
            }
        }
        self.items = items;
        self.track = first_track;
        self.generation = self.generation.wrapping_add(1);
        Ok(())
    }
}

struct QueueEndHandler {
    state: Arc<Mutex<Player>>,
    queue: TrackQueue,
    manager: Arc<Songbird>,
    guild_id: GuildId,
    on_error: bool,
}

#[async_trait]
impl VoiceEventHandler for QueueEndHandler {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        let mut state = self.state.lock().await;
        let ended_uuid = match ctx {
            EventContext::Track(tracks) => tracks.first().map(|(_, track)| track.uuid().as_u128()),
            _ => None,
        }?;
        if state.items.front().and_then(|item| item.track_uuid) != Some(ended_uuid) {
            return None;
        }
        if self.on_error {
            let queue = self.queue.clone();
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                if queue
                    .current()
                    .is_some_and(|current| current.uuid().as_u128() == ended_uuid)
                {
                    let _ = queue.skip();
                }
            });
            return None;
        }
        state.items.pop_front();
        state.track = self.queue.current();
        if state.looping {
            if let Some(track) = &state.track {
                let _ = track.enable_loop();
            }
        }
        if state.items.is_empty() {
            let generation = state.generation;
            schedule_disconnect(
                self.manager.clone(),
                self.state.clone(),
                self.guild_id,
                generation,
            );
        }
        None
    }
}

fn add_track_events(
    handle: &TrackHandle,
    state: Arc<Mutex<Player>>,
    queue: TrackQueue,
    manager: Arc<Songbird>,
    guild_id: GuildId,
) -> Result<(), String> {
    handle
        .add_event(
            Event::Track(TrackEvent::End),
            QueueEndHandler {
                state: state.clone(),
                queue: queue.clone(),
                manager: manager.clone(),
                guild_id,
                on_error: false,
            },
        )
        .map_err(|e| e.to_string())?;
    handle
        .add_event(
            Event::Track(TrackEvent::Error),
            QueueEndHandler {
                state,
                queue,
                manager,
                guild_id,
                on_error: true,
            },
        )
        .map_err(|e| e.to_string())
}

fn schedule_disconnect(
    manager: Arc<Songbird>,
    state: Arc<Mutex<Player>>,
    guild_id: GuildId,
    generation: u64,
) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let mut player = state.lock().await;
        if player.generation == generation && player.items.is_empty() {
            let _ = manager.remove(guild_id).await;
            player.track = None;
            player.channel_id = None;
        }
    });
}
