extern crate dotenv;
use dotenv::dotenv;

use futures::StreamExt;
use pot::SystemPlaylist;
use songbird::{
    shards::TwilightMap,
    tracks::{TrackHandle},
    Songbird,
};
use std::{collections::HashMap, env, error::Error, sync::Arc};
use tokio::sync::RwLock;
use twilight_gateway::{
    stream::{self, ShardEventStream},
    Event,
    Intents,
    Shard,
};
use twilight_http::Client as HttpClient;
use twilight_model::{
    id::{marker::{GuildMarker, ApplicationMarker, UserMarker}, Id},
};
use twilight_cache_inmemory::InMemoryCache;
use twilight_standby::Standby;

mod interaction;
mod helpers;
mod yt;
mod pot;
mod colour;

#[derive(Debug)]
pub struct StateRef {
    http: HttpClient,
    trackdata: RwLock<HashMap<Id<GuildMarker>, TrackHandle>>,
    system_playlist: Arc<RwLock<SystemPlaylist>>,
    songbird: Arc<Songbird>,
    standby: Standby,
    application_id: Id<ApplicationMarker>,
    bot_id: Id<UserMarker>,
    cache: InMemoryCache
}

async fn update_global_commands(info: Arc<StateRef>) -> anyhow::Result<(usize, usize)> {
    let client = info.http.interaction(info.application_id);
    let globals = client.global_commands().await?.model().await?;

    // let guilds = info.http.current_user_guilds().await?;

    let mut deleted = 0;
    for global in globals.iter() {
        deleted += 1;
        client.delete_global_command(global.id.unwrap()).await?;
    }

    let list = client
        .set_global_commands(&interaction::CREATE_GLOBAL_COMMANDS)
        .await?
        .model()
        .await?;
    Ok((list.len(), deleted))
}

// async fn update_guild_commands(info: Arc<StateRef>) -> anyhow::Result<usize> {
//     let client = info.http.interaction(info.application_id);
//     let guilds = info.http.current_user_guilds().await?.model().await?;

//     for guild in guilds.iter() {
//         println!("guild id {} name {}", guild.id, guild.name);

//         let guild_commands = client.guild_commands(guild.id).await?.model().await?;
//         println!("guild {} has {} commands", guild.name, guild_commands.len());

//         println!("Deleting commands in {}", guild.name);
//         let mut deleted = 0;
//         for command in guild_commands.iter() {
//             deleted += 1;
//             println!(" > deleting {} command", command.name);
//             client.delete_guild_command(guild.id, command.id.unwrap()).await?;
//         }

//         let list = client
//             .set_guild_commands(guild.id, &interaction::CREATE_GUILD_COMMANDS)
//             .await?
//             .model()
//             .await?;

//         println!("Updated {} commands and deleted {} commands in guild {}.", list.len(), deleted, guild.id);
//     }
    
//     Ok(guilds.len())
// }

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync + 'static>> {
    dotenv().ok();

    // Initialize the tracing subscriber.
    tracing_subscriber::fmt::init();

    std::env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN env var");
    std::env::var("YOUTUBE_TOKEN").expect("Missing YOUTUBE_TOKEN env var");

    // Setup dir structure
    match helpers::setup_system() {
        Ok(_) => println!("Directories setup complete"),
        Err(err) => {
            panic!("{:?}", err);
        },
    }

    let (mut shards, state) = {
        let token = env::var("DISCORD_TOKEN")?;

        let http = HttpClient::new(token.clone());
        let user_id = http.current_user().await?.model().await?.id;

        let intents =
            Intents::GUILDS | Intents::GUILD_MESSAGES | Intents::GUILD_VOICE_STATES | Intents::MESSAGE_CONTENT;
        let config = twilight_gateway::Config::new(token.clone(), intents);

        let shards: Vec<Shard> =
            stream::create_recommended(&http, config, |_, builder| builder.build())
                .await?
                .collect();

        let senders = TwilightMap::new(
            shards
                .iter()
                .map(|s| (s.id().number(), s.sender()))
                .collect(),
        );

        let application_id = {
            let response = http.current_user_application().await?;
            response.model().await?.id
        };

        let bot_id = {
            let response = http.current_user().await?;
            response.model().await?.id
        };

        let cache = InMemoryCache::builder()
            .message_cache_size(10)
            .build();

        let songbird = Songbird::twilight(Arc::new(senders), user_id);
        let system_playlist = Arc::new(RwLock::new(SystemPlaylist::new()));

        (
            shards,
            Arc::new(StateRef {
                http,
                trackdata: Default::default(),
                system_playlist: system_playlist.clone(),
                songbird: Arc::new(songbird),
                standby: Standby::new(),
                application_id,
                bot_id,
                cache
            })
        )
    };

    // {
    //     let (updated, deleted) = update_global_commands(state.clone()).await?;
    //     println!("Updated {updated} global commands and deleted {deleted} global commands.");
    // }

    // {
    //     let guilds_updated = update_guild_commands(state.clone()).await?;
    //     println!("Updated commands in {guilds_updated} guilds.");
    // }

    let mut stream = ShardEventStream::new(shards.iter_mut());
    loop {
        let event = match stream.next().await {
            Some((_, Ok(event))) => event,
            Some((_, Err(source))) => {
                tracing::warn!(?source, "error receiving event");

                if source.is_fatal() {
                    break;
                }

                continue;
            },
            None => break,
        };

        state.standby.process(&event);
        state.songbird.process(&event).await;

        match &event {
            Event::MessageCreate(msg) => {
                if msg.guild_id.is_none() || !msg.content.starts_with('!') {
                    continue;
                }
            },
            Event::InteractionCreate(interaction) => {
                let handler = interaction::handle_interaction(interaction.clone(), state.clone()).await;
                if let Err(err) = handler {
                    eprintln!(
                        "Error found on interaction {}\nError: {:?}",
                        interaction.id, err
                    );
                }
            },
            _ => {}
        }

        state.cache.update(&event);
    }
    
    Ok(())
}
