mod commands;
mod util;

use poise::serenity_prelude as serenity;
use serenity::{
    all::{ClientBuilder, GatewayIntents, Message, Ready},
    async_trait,
    client::{Context, EventHandler},
};
use songbird::SerenityInit;
use std::{collections::HashMap, env, sync::Arc};
use tokio::sync::Mutex;
use tracing::{error, info};
use util::{media::MediaSettings, player::Player, youtube::YoutubeApi};

pub type PError = Box<dyn std::error::Error + Send + Sync>;
pub type PContext<'a> = poise::Context<'a, Data, PError>;

pub struct Data {
    pub youtube: YoutubeApi,
    pub media: MediaSettings,
    pub voice: Arc<songbird::Songbird>,
    players: Mutex<HashMap<serenity::GuildId, Arc<Mutex<Player>>>>,
}

impl Data {
    pub async fn player_state(&self, guild_id: serenity::GuildId) -> Arc<Mutex<Player>> {
        self.players
            .lock()
            .await
            .entry(guild_id)
            .or_default()
            .clone()
    }
}

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _ctx: Context, ready: Ready) {
        info!("Connected as {}", ready.user.name);
    }

    async fn message(&self, ctx: Context, message: Message) {
        if message.author.bot || message.guild_id.is_none() || !message.content.starts_with('!') {
            return;
        }
        let response = if message.content == "!register" {
            "Successfully registered application commands."
        } else {
            "❌ Please use the new slash (/) commands."
        };
        if let Err(error) = message.channel_id.say(&ctx.http, response).await {
            error!("Could not reply to legacy prefix command: {error}");
        }
    }
}

async fn on_error(error: poise::FrameworkError<'_, Data, PError>) {
    if let poise::FrameworkError::CommandCheckFailed { ctx, .. } = &error {
        let _ = ctx.send(poise::CreateReply::default().content(":x: **You have to be in a voice channel in this guild to use this command.**").ephemeral(true)).await;
        return;
    }
    if let poise::FrameworkError::Command { error, ctx, .. } = &error {
        error!("Command {} failed: {error}", ctx.command().name);
    }
    if let Err(error) = poise::builtins::on_error(error).await {
        error!("Failed to respond to command error: {error}");
    }
}

#[tokio::main]
async fn main() -> Result<(), PError> {
    tracing_subscriber::fmt::init();
    rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
        .map_err(|_| "Could not install the Rustls ring crypto provider")?;

    let mut token = env::var("DISCORD_APP_AUTH_TOKEN").ok();
    let mut youtube_key = env::var("YOUTUBE_API_KEY").ok();
    for argument in env::args().skip(1) {
        if let Some(value) = argument.strip_prefix("--token=") {
            token = Some(value.to_owned());
        } else if let Some(value) = argument.strip_prefix("--key=") {
            youtube_key = Some(value.to_owned());
        }
    }
    let token = token.ok_or("Please set DISCORD_APP_AUTH_TOKEN or pass --token=")?;
    let youtube_key = youtube_key.ok_or("Please set YOUTUBE_API_KEY or pass --key=")?;
    let intents = GatewayIntents::GUILDS
        | GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::GUILD_VOICE_STATES
        | GatewayIntents::MESSAGE_CONTENT;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: commands::all(),
            command_check: Some(|ctx| {
                Box::pin(async move {
                    let in_voice = ctx
                        .guild()
                        .is_some_and(|guild| guild.voice_states.contains_key(&ctx.author().id));
                    Ok(in_voice)
                })
            }),
            on_error: |error| Box::pin(on_error(error)),
            pre_command: |ctx| {
                Box::pin(async move {
                    info!("Running /{}", ctx.command().qualified_name);
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            Box::pin(async move {
                poise::builtins::register_globally(ctx, &framework.options().commands).await?;
                let voice = songbird::get(ctx)
                    .await
                    .expect("Songbird voice manager is registered")
                    .clone();
                Ok(Data {
                    youtube: YoutubeApi::new(youtube_key),
                    media: MediaSettings::from_environment(),
                    voice,
                    players: Mutex::new(HashMap::new()),
                })
            })
        })
        .build();

    let mut client = ClientBuilder::new(token, intents)
        .event_handler(Handler)
        .framework(framework)
        .register_songbird()
        .await?;

    let shard_manager = client.shard_manager.clone();
    tokio::spawn(async move {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!("Could not install Ctrl+C handler: {error}");
            return;
        }
        shard_manager.shutdown_all().await;
    });

    if let Ok(healthcheck_url) = env::var("HEALTHCHECK_URL") {
        tokio::spawn(async move {
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
            {
                Ok(client) => client,
                Err(error) => {
                    tracing::error!("Could not create healthcheck client: {error}");
                    return;
                }
            };
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            loop {
                interval.tick().await;
                if let Err(error) = client
                    .get(&healthcheck_url)
                    .send()
                    .await
                    .and_then(reqwest::Response::error_for_status)
                {
                    tracing::warn!("Healthcheck ping failed: {error}");
                }
            }
        });
    }

    if let Err(error) = client.start().await {
        error!("Discord client stopped: {error}");
    }
    Ok(())
}
