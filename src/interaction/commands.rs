use std::sync::Arc;
use anyhow::{Result};
use async_recursion::async_recursion;
use tokio::sync::{Mutex, RwLock, RwLockWriteGuard};
use songbird::{
    Songbird,
    id::{ChannelId, GuildId},
    Call, Event, EventContext, EventHandler as VoiceEventHandler, TrackEvent
};
use twilight_interactions::command::{CommandModel, CreateCommand};
use twilight_model::{
    application::interaction::Interaction, 
    http::interaction::{
        InteractionResponseType, 
        InteractionResponse
    }, 
    channel::message::{
        MessageFlags
    }, id::{marker::{InteractionMarker, ApplicationMarker, GuildMarker, ChannelMarker}, Id}
};
use twilight_util::builder::{InteractionResponseDataBuilder, embed::{EmbedBuilder, ImageSource, EmbedFooterBuilder}};
use url::Url;

use crate::{StateRef, pot::{PotPlayInputType, PlaylistItem, SystemPlaylist}, colour::Colour};
use async_trait::async_trait;

pub struct TrackEndNotifier {
    state: Arc<StateRef>,
    channel_id: Id<ChannelMarker>,
    guild_id: Id<GuildMarker>,
    call: Arc<Mutex<Call>>,
    playlist: Arc<RwLock<SystemPlaylist>>,
    manager: Arc<Songbird>
}

#[async_trait]
impl VoiceEventHandler for TrackEndNotifier {
    async fn act(&self, ctx: &EventContext<'_>) -> Option<Event> {
        if let EventContext::Track(_track_list) = ctx {
            let mut handler: tokio::sync::MutexGuard<Call> = self.call.lock().await;
            let mut playlist = self.playlist.write().await;
            
            if consume_and_play_on_end(self, &mut handler, &mut playlist).await.is_none() {
                // let _ = self.channel_id.say(&self.ctx.http(), "Queue finished").await;
                let _ = send_queue_finished(&self.state.http, self.channel_id).await;
                // let _ = self.channel_id.say(&self.ctx.http(), "Left voice channel").await;
                drop(handler);
                let _ = self.manager.remove(self.guild_id).await;
            }
        }

        None
    }
}

#[derive(CommandModel, CreateCommand)]
#[command(name = "join", desc = "Join to voice channel")]
pub struct JoinCommand;

enum JoinResult {
    Ok(String),
    Err(String)
}

impl From<JoinResult> for String {
    fn from(value: JoinResult) -> Self {
        match value {
            JoinResult::Ok(str) => str,
            JoinResult::Err(str) => str,
        }
    }
}

impl JoinCommand {
    pub async fn run(self, state: Arc<StateRef>, interaction: Interaction, direct: bool) -> Result<Option<Arc<Mutex<Call>>>> {
        let guild_id: Id<GuildMarker>;

        match interaction.guild_id {
             // Get guild id of the interaction
            Some(guild_id_ex) => guild_id = guild_id_ex,
            None => {
                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "This command only works in guilds").await?;
                return Ok(None)
            },
        }

        let interaction_channel = interaction.clone().channel.unwrap().id;

        // Get the bot call on the guild
        match state.songbird.get(guild_id) {
            Some(call) => {
                // If the call exist we tell the user that the bot is already in a call on the guild and do nothing more
                if !direct {
                    send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "Already in voice channel").await?;
                }
                return Ok(Some(call))
            },
            None => {
                // If the call not exist we get the data from the interaction and join the caller voice channel

                // Get the caller user id
                let author_id = interaction.author_id().unwrap();
                // Get the voice state of the user
                let voice_state = state.cache.voice_state(author_id, interaction.guild_id.unwrap());

                // Check if the voice state exists and return a message
                let response: (JoinResult, Option<Arc<Mutex<Call>>>) = if let Some(voice_state) = &voice_state {
                    // We get the user's current voice channel
                    let author_channel = voice_state.channel_id().clone();

                    // Then we try to join the voice channel and return a message
                    match state.songbird.join(guild_id, author_channel).await {
                        Ok(call_lock) => {
                            let mut call = call_lock.lock().await;
                            call.add_global_event(
                                Event::Track(TrackEvent::End),
                                TrackEndNotifier {
                                    state: state.clone(),
                                    channel_id: interaction_channel,
                                    guild_id,
                                    call: call_lock.clone(),
                                    playlist: state.system_playlist.clone(),
                                    manager: state.songbird.clone(),
                                },
                            );
                            drop(call);

                            (JoinResult::Ok(format!("Joined <#{}>!", author_channel)), Some(call_lock))
                        },
                        Err(e) => (JoinResult::Err(format!("Failed to join <#{}>! Why: {:?}", author_channel, e)), None),
                    }
                } else {
                    // The user was not in a voice channel and we just return a message
                    (JoinResult::Err("User not in a voice channel".into()), None)
                };
                
                let response_str: String = response.0.into();
                if !direct {
                    send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, &response_str).await?;
                } else {
                    let _ = send_message(&state.http, interaction_channel, &response_str);
                }

                if direct {
                    return Ok(response.1)
                }
            },
        }

        Ok(None)
    }
}

#[derive(CommandModel, CreateCommand)]
#[command(name = "leave", desc = "Leave voice channel")]
pub struct LeaveCommand;

impl LeaveCommand {
    pub async fn run(self, state: Arc<StateRef>, interaction: Interaction) -> Result<()> {
        let guild_id: Id<GuildMarker>;

        match interaction.guild_id {
             // Get guild id of the interaction
            Some(guild_id_ex) => guild_id = guild_id_ex,
            None => {
                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "This command only works in guilds").await?;
                return Ok(())
            },
        }

        // Get the bot call on the guild
        match state.songbird.get(guild_id) {
            Some(call) => {
                // The bot is in a voice channel, we need to check if the user is in the same voice channel as the bot

                // Get the user id
                let author_id = interaction.author_id().unwrap();
                // Get the voice state of the user
                let voice_state = state.cache.voice_state(author_id, interaction.guild_id.unwrap());


                let response: String = if let Some(voice_state) = &voice_state {
                    // The user is in a voice channel now we need to check if the user is in the same voice channel as the bot

                    let call = call.lock().await;

                    // Get user and bot voice channel id in songbird format
                    let author_channel: ChannelId = voice_state.channel_id().clone().into();
                    let bot_channel = call.current_channel().unwrap();

                    if author_channel == bot_channel {
                        // The user is in the same channel as the bot, we leave the call
                        
                        // Drop the call
                        drop(call);

                        let mut playlist = state.system_playlist.write().await;

                        playlist.clear(&guild_id);
                        playlist.set_status(&guild_id, false);

                        drop(playlist);

                        // Leave the call
                        let _ = state.songbird.remove(guild_id).await;

                        // Return message
                        "Disconnected".into()
                    } else {
                        // The user is not in the same voice channel and we just return a message
                        "User not in the channel".into()
                    }
                } else {
                    // The user was not in a voice channel and we just return a message
                    "User not in a voice channel".into()
                };

                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, &response).await?;
            },
            None => {
                // The bot is not in a voice call we just send a message
                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "Not in voice channel").await?;
            },
        }

        Ok(())
    }
}

#[derive(CommandModel, CreateCommand)]
#[command(name = "play", desc = "Play song")]
pub struct PlayCommand {
    /// Message to send
    song: String
}

impl PlayCommand {
    pub async fn run(self, state: Arc<StateRef>, interaction: Interaction) -> Result<()> {
        let guild_id: Id<GuildMarker>;

        match interaction.guild_id {
             // Get guild id of the interaction
            Some(guild_id_ex) => guild_id = guild_id_ex,
            None => {
                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "This command only works in guilds").await?;
                return Ok(())
            },
        }

        let interaction_channel_id = interaction.channel.clone().unwrap().id;

        let author_name = interaction.clone().author().unwrap().clone().name;
        let author_id = interaction.author_id().unwrap();
        let avatar_hash: String = if let Some(hash) = interaction.author().unwrap().avatar {
            hash.to_string()
        } else { String::new() };
        let avatar_url = format!("https://cdn.discordapp.com/avatars/{author_id}/{avatar_hash}.webp?size=40");

        let voice_state = state.cache.voice_state(author_id, guild_id);

        let response_message: String = match &voice_state {
            Some(_) => "Adding...".into(),
            None => "Not in a voice channel".into(),
        };

        send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, &response_message).await?;

        if let Some(_voice_state) = &voice_state {
            let join_command = JoinCommand;
            match join_command.run(state.clone(), interaction, true).await {
                Ok(join_result) => {
                    if let Some(call) = join_result {
                        // Get pot input type from src
                        let input = match Url::parse(&self.song) {
                            Ok(url_parsed) => {
                                if url_parsed.host_str().unwrap_or("").ends_with("open.spotify.com") {
                                    PotPlayInputType::SpotifyUrl(url_parsed)
                                } else {
                                    PotPlayInputType::Url(url_parsed)
                                }
                            },
                            Err(_) => PotPlayInputType::Search(self.song.clone())
                        };

                        // Get playlist
                        let mut playlist = state.system_playlist.write().await;
                        let mut call_lock = call.lock().await;
    
                        // let channel_id = call_lock.current_channel().unwrap();
                        // let channel_id: Id<ChannelMarker> = Id::new(channel_id.0.into());
                        
                        match playlist.add(&guild_id, input).await {
                            Ok((items_added_count, items_slice)) => {
                                if items_added_count > 1 {
                                    let _ = send_playlist_added(&state.http, interaction_channel_id, &author_name, &avatar_url, items_slice).await;
                                } else {
                                    let _ = send_song_added(&state.http, interaction_channel_id, &author_name, &avatar_url, items_slice.first().unwrap()).await;
                                }
                
                                if !playlist.is_playing(&guild_id) && consume_and_play(&state.http, interaction_channel_id, &mut playlist, guild_id, &mut call_lock).await.is_none(){
                                    let _ = state.songbird.remove(guild_id).await;
                                    let _ = send_message(&state.http, interaction_channel_id, "Left voice channel").await;
                                }
                                drop(call_lock);
                                drop(playlist);
                            },
                            Err(_err) => {
                                let _ = send_message(&state.http, interaction_channel_id, "Error adding to the playlist").await;
                            }
                        }
                    } else {
                        println!("No call obtained");
                    }
                },
                Err(join_error) => {
                    println!("No joined fail {join_error:?}");
                },
            }
        } else {
            println!("No voice state");
        }

        Ok(())
    }
}

#[derive(CommandModel, CreateCommand)]
#[command(name = "skip", desc = "Skip song")]
pub struct SkipCommand;

impl SkipCommand {
    pub async fn run(self, state: Arc<StateRef>, interaction: Interaction) -> Result<()> {
        let guild_id: Id<GuildMarker>;

        match interaction.guild_id {
             // Get guild id of the interaction
            Some(guild_id_ex) => guild_id = guild_id_ex,
            None => {
                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "This command only works in guilds").await?;
                return Ok(())
            },
        }

        let interaction_channel_id = interaction.channel.clone().unwrap().id;

        // Get the bot call on the guild
        match state.songbird.get(guild_id) {
            Some(call) => {
                // The bot is in a voice channel, we need to check if the user is in the same voice channel as the bot

                // Get the user id
                let author_id = interaction.author_id().unwrap();
                // Get the voice state of the user
                let voice_state = state.cache.voice_state(author_id, interaction.guild_id.unwrap());


                let response: String = if let Some(voice_state) = &voice_state {
                    // The user is in a voice channel now we need to check if the user is in the same voice channel as the bot

                    let mut call = call.lock().await;

                    // Get user and bot voice channel id in songbird format
                    let author_channel: ChannelId = voice_state.channel_id().clone().into();
                    let bot_channel = call.current_channel().unwrap();

                    if author_channel == bot_channel {
                        // The user is in the same channel as the bot, we leave the call
                        let mut playlist = state.system_playlist.write().await;

                        let result = song_skip(state.songbird.clone(), &state.http, interaction_channel_id, &mut playlist, guild_id, &mut call).await;

                        // Drop the call
                        drop(call);
                        drop(playlist);

                        match result {
                            Ok(message) =>message,
                            Err(error) => "Something happened D:".into(),
                        }
                    } else {
                        // The user is not in the same voice channel and we just return a message
                        "User not in the channel".into()
                    }
                } else {
                    // The user was not in a voice channel and we just return a message
                    "User not in a voice channel".into()
                };

                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, &response).await?;
            },
            None => {
                // The bot is not in a voice call we just send a message
                send_response(&state.http, interaction.application_id, interaction.id, &interaction.token, "Not in voice channel").await?;
            },
        }

        Ok(())
    }
}

// pub async fn defer_reply(
//     info: Arc<StateRef>,
//     interaction: &Interaction,
//     builder: InteractionResponseDataBuilder,
// ) -> Result<()> {
//     info.http
//         .interaction(info.application_id)
//         .create_followup(&interaction.token).content(content)
//         .await?;

//     Ok(())
// }

async fn send_response(
    http: &twilight_http::Client,
    application_id: Id<ApplicationMarker>,
    interaction_id: Id<InteractionMarker>,
    interaction_token: &str,
    response: &str
) -> Result<()> {
    let interaction_response_data = InteractionResponseDataBuilder::new()
        .content(response)
        .flags(MessageFlags::EPHEMERAL)
        .build();


    http
        .interaction(application_id)
        .create_response(interaction_id, interaction_token, &InteractionResponse {
            kind: InteractionResponseType::ChannelMessageWithSource,
            data: Some(interaction_response_data),
        })
        .await?;

    Ok(())
}

#[async_recursion]
async fn consume_and_play(
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
    playlist: &mut SystemPlaylist, 
    guild_id: Id<GuildMarker>, 
    call: &mut tokio::sync::MutexGuard<'_, Call>
) -> Option<()> {
    // Try to consume a item from the playlist
    match playlist.consume(&guild_id) {
        Some(playlist_item) => {
            // If we found a PlaylistItem available we change the playlist status to playing
            playlist.set_status(&guild_id, true);
            
            // Then we try to get the mefia file
            match playlist.get_media(&playlist_item).await {
                Ok(source) => {
                    // Send message to channel

                    // Play the source
                    let _ = call.play_only_input(source.into());

                    let _ = send_now_playing(&http, channel_id, &playlist_item).await;
                    Some(())
                },
                Err(err) => {
                    println!("{:?}", err);
                    // Set status to not playing
                    playlist.set_status(&guild_id, false);
                    // Send message of error
                    let _ = send_message(http, channel_id, &format!("Cannot play {}", playlist_item.title)).await;
                    // Try again
                    consume_and_play(&http, channel_id, playlist, guild_id, call).await
                }
            }
        },
        None => {
            // No more items in playlist
            // let _ = channel_id.say(&http, "Queue finished").await;
            let _ = send_queue_finished(&http, channel_id).await;
            // Set status to not playing
            playlist.set_status(&guild_id, false);
            None
        }
    }
}

#[async_recursion]
pub async fn consume_and_play_on_end (
    slf: &TrackEndNotifier, 
    call: &mut tokio::sync::MutexGuard<'_, Call>, 
    playlist: &mut RwLockWriteGuard<SystemPlaylist>
) -> Option<()> {
    match playlist.consume(&slf.guild_id) {
        Some(item) => {
            println!("consumed");
            match playlist.get_media(&item).await {
                Ok(source) => {
                    println!("media getted");

                    call.play_only_input(source.into());
                    let _ = send_now_playing_on_end(&slf, &item).await;
                    Some(())
                },
                Err(err) => {
                    println!("{:?}", err);
                    println!("media not getted");
                    let _ = send_cannot_play_on_end(&slf, &item).await;
                    consume_and_play_on_end(slf, call, playlist).await
                },
            }
        },
        None => {
            playlist.set_status(&slf.guild_id, false);
            None
        },
    }
}

pub async fn song_skip(
    songbird: Arc<Songbird>,
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
    playlist: &mut SystemPlaylist, 
    guild_id: Id<GuildMarker>, 
    call: &mut tokio::sync::MutexGuard<'_, Call>
) -> Result<String> {
    call.stop();

    if playlist.is_playing(&guild_id) {
        if consume_and_play(http, channel_id, playlist, guild_id, call).await.is_none() {
            drop(call);
            let _ = songbird.remove(guild_id).await;
            Ok("Queue ended".into())
        } else {
            Ok("Song skipped".into())
        }
    } else {
        Ok("Nothing to play".into())
    }
}

async fn send_message(
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
    message: &str
) -> Result<()> {
    http
        .create_message(channel_id)
        .content(message).unwrap()
        .await?;

    Ok(())
}

async fn send_playlist_added(
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
    user_name: &str,
    avatar_url: &str,
    items: &[PlaylistItem]
) -> Result<()> {

    let footer = EmbedFooterBuilder::new(format!("Requested by {}", user_name))
        .icon_url(ImageSource::url(avatar_url).unwrap())
        .build();

    let embed = EmbedBuilder::new()
        .title(":musical_note:  **Playlist added to queue**")
        .description(format!("{} elements added to playlist", &items.len()))
        .footer(footer)
        .build();

    http
        .create_message(channel_id)
        .embeds(&[
            embed
        ]).unwrap()
        .await?;

    Ok(())
}

async fn send_song_added(
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
    user_name: &str,
    avatar_url: &str,
    item: &PlaylistItem
) -> Result<()> {
    let thumbnail = item.thumbnail.as_ref().unwrap_or(&String::new()).to_owned();

    let footer = EmbedFooterBuilder::new(format!("Requested by {}", user_name))
        .icon_url(ImageSource::url(avatar_url).unwrap())
        .build();

    let embed = EmbedBuilder::new()
        .title(":musical_note:  **Song added to queue**")
        .description(format!("[{}]({})", &item.title, &item.original_url))
        .thumbnail(ImageSource::url(thumbnail).unwrap())
        .footer(footer)
        .build();

    http
        .create_message(channel_id)
        .embeds(&[
            embed
        ]).unwrap()
        .await?;

    Ok(())
}


pub async fn send_now_playing(
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
    item: &PlaylistItem
) {
    let embed = EmbedBuilder::new()
        .title(":musical_note:  **Now playing**")
        .description(format!("[{}]({})", &item.title, &item.original_url))
        .thumbnail(ImageSource::url(&item.thumbnail.clone().unwrap_or("".into())).unwrap())
        .color(Colour::GOLD.0)
        .build();

    let _ = http
        .create_message(channel_id)
        .embeds(&[
            embed
        ]).unwrap()
        .await;
}



pub async fn send_queue_finished(
    http: &twilight_http::Client,
    channel_id: Id<ChannelMarker>,
) {
    let embed = EmbedBuilder::new()
        .title(":musical_note:  **Queue finished**")
        .color(Colour::DARK_GREY.0)
        .build();

    let _ = http
        .create_message(channel_id)
        .embeds(&[
            embed
        ]).unwrap()
        .await;
}

pub async fn send_now_playing_on_end(slf: &TrackEndNotifier, item: &PlaylistItem) {
    let embed = EmbedBuilder::new()
        .title(":musical_note:  **Now playing**")
        .description(format!("[{}]({})", &item.title, &item.original_url))
        .thumbnail(ImageSource::url(&item.thumbnail.clone().unwrap_or("".into())).unwrap())
        .color(Colour::GOLD.0)
        .build();

    let _ = slf.state.http
        .create_message(slf.channel_id)
        .embeds(&[
            embed
        ]).unwrap()
        .await;
}

pub async fn send_cannot_play_on_end(slf: &TrackEndNotifier, item: &PlaylistItem) {
    let embed = EmbedBuilder::new()
        .title(":musical_note:  **Cannot play**")
        .description(format!("[{}]({})", &item.title, &item.original_url))
        .thumbnail(ImageSource::url(&item.thumbnail.clone().unwrap_or("".into())).unwrap())
        .color(Colour::RED.0)
        .build();

    let _ = slf.state.http
        .create_message(slf.channel_id)
        .embeds(&[
            embed
        ]).unwrap()
        .await;
}