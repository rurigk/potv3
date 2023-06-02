use anyhow::{anyhow};
use serde::{Deserialize};
use twilight_model::id::Id;
use twilight_model::id::marker::GuildMarker;
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, BufRead};
use std::path::Path;
use std::process::ChildStdout;
use std::{
    io::{Read},
    process::{Command, Stdio},
};


#[cfg(not(feature = "tokio-02-marker"))]
use tokio::{task};
#[cfg(feature = "tokio-02-marker")]
use tokio_compat::{task};

use crate::helpers;
use crate::yt::YoutubeResult;


#[derive(Debug, Deserialize, Clone, Copy)]
pub enum YOUTUBE_DL_BACKEND {
    YT_DLP,
    YOUTUBE_DL
}

impl YOUTUBE_DL_BACKEND {
    pub fn value (&self) -> &str {
        match self {
            YOUTUBE_DL_BACKEND::YT_DLP => "yt-dlp",
            YOUTUBE_DL_BACKEND::YOUTUBE_DL => "youtube-dl",
        }
    }
}

#[derive(Debug)]
pub struct SystemPlaylist {
    guilds_playlists: HashMap<Id<GuildMarker>, Vec<PlaylistItem>>,
    guilds_playing: HashMap<Id<GuildMarker>, bool>
}

#[derive(Debug)]
pub enum PotPlayInputType {
    Url(url::Url),
    SpotifyUrl(url::Url),
    Search(String)
}

impl PotPlayInputType {
    fn is_url(&self) -> bool {
        matches!(*self, Self::Url(_))
    }
}

#[derive(Debug)]
enum YoutubeUrlType {
    Video(String),
    Playlist(String),
    Short(String),
    None
}

fn youtube_url_extractor (url: &url::Url) -> YoutubeUrlType {
    match url.host_str() {
        Some(url_str) => {
            let path_segments = url
                    .path_segments()
                    .map(|c| c.collect::<Vec<_>>()).unwrap_or_default();

            if url_str.ends_with("youtube.com") || url_str.ends_with("youtu.be") {
                let query = query_pairs_to_hashmap(url);

                if query.contains_key("list") {
                    YoutubeUrlType::Playlist(query.get("list").unwrap().to_owned())
                } else if query.contains_key("v") {
                    YoutubeUrlType::Video(query.get("v").unwrap().to_owned())
                } else if path_segments[0] == "shorts" {
                    YoutubeUrlType::Short(path_segments[1].to_string())
                } else {
                    YoutubeUrlType::None
                }
            } else {
                YoutubeUrlType::None
            }
        },
        None => YoutubeUrlType::None,
    }
}

enum SpotifyUrlType {
    Track(String),
    Playlist(String),
    None
}

fn spotify_url_extractor (url: &url::Url) -> SpotifyUrlType {
    match url.host_str() {
        Some(url_str) => {
            if url_str.ends_with("open.spotify.com") {
                let path_segments = url
                    .path_segments()
                    .map(|c| c.collect::<Vec<_>>()).unwrap_or_default();

                if path_segments.len() < 2 { return SpotifyUrlType::None }

                if path_segments[0] == "playlist" {
                    SpotifyUrlType::Playlist(path_segments[1].to_owned())
                } else if path_segments[0] == "track" {
                    SpotifyUrlType::Track(path_segments[1].to_owned())
                } else {
                    SpotifyUrlType::None
                }
            } else {
                SpotifyUrlType::None
            }
        },
        None => SpotifyUrlType::None,
    }
}

fn query_pairs_to_hashmap (url: &url::Url) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for (key, value) in url.query_pairs() {
        let qkey = key.to_string();
        let qvalue = value.to_string();
        map.entry(qkey).or_insert(qvalue);
    }
    map
}

fn youtube_result_to_playlist_items (yt_result: YoutubeResult) -> Vec<PlaylistItem> {
    if let YoutubeResult::Ok(response) = yt_result {
        response.items.into_iter().filter_map(|item| {
            if let Some(resource_id) = item.snippet.resourceId {
                Some(PlaylistItem {
                    original_url: format!("https://www.youtube.com/watch?v={}", &resource_id.videoId),
                    id: resource_id.videoId,
                    title: item.snippet.title,
                    extractor: "youtube".to_string(),
                    thumbnail: item.snippet.thumbnails.get("default").map(|t| t.url.to_owned()),
                    duration: None,
                    playlist_id: None,
                    webpage_url: None,
                    is_live: None,
                    was_live: None,
                    backend: Some(YOUTUBE_DL_BACKEND::YT_DLP)
                })
            } else {
                None
            }
        }).collect()
    } else {
        Vec::new()
    }
}

impl SystemPlaylist {
    pub fn new () -> Self {
        Self {
            guilds_playlists: HashMap::new(),
            guilds_playing: HashMap::new()
        }
    }

    pub fn set_status (&mut self, guild_id: &Id<GuildMarker>, is_playing: bool) {
        if self.guilds_playing.contains_key(guild_id) {
            let guild_playlist_status = self.guilds_playing.get_mut(guild_id).unwrap();
            *guild_playlist_status = is_playing;

            println!("status set :{}", guild_playlist_status);
        } else {
            self.guilds_playing.insert(guild_id.to_owned(), is_playing);
            println!("status set :{}", is_playing);
        }
    }

    pub fn is_playing (&self, guild_id: &Id<GuildMarker>) -> bool {
        if self.guilds_playing.contains_key(guild_id) {
            *self.guilds_playing.get(guild_id).unwrap()
        } else {
            false
        }
    }

    /// Consumes and return a item from the the guild playlist removing the item
    pub fn consume(&mut self, guild_id: &Id<GuildMarker>) -> Option<PlaylistItem> {
        if self.guilds_playlists.contains_key(guild_id) { // Guild playlist already exist
            let guild_playlist = self.guilds_playlists.get_mut(guild_id).unwrap();
            
            if guild_playlist.is_empty() {
                None
            } else {
                Some(guild_playlist.remove(0))
            }
        } else { // The guild playlist is not currently in the system
            None
        }
    }

    /// Try to fetch a playlist or a single media item and add it to the guild playlist
    pub async fn add(&mut self, guild_id: &Id<GuildMarker>, input: PotPlayInputType) -> anyhow::Result<(usize, &[PlaylistItem])> {
        use crate::yt::YoutubeAPI;

        // Load youtube token
        let token = std::env::var("YOUTUBE_TOKEN").expect("missing YOUTUBE_TOKEN");

        // Initialize Youtube api
        let api = YoutubeAPI::new(&token);

        // Check if the input is a url or a query
        let is_url = input.is_url();

        // Get a PlaylistItem vec

        let playlist_result = match input {
            PotPlayInputType::Url(url) => {
                // Check if the url is a youtube url
                match youtube_url_extractor (&url) {
                    YoutubeUrlType::Playlist(playlist_id) => Ok(youtube_result_to_playlist_items(api.playlist(&playlist_id).await)),
                    YoutubeUrlType::Video(video_id) => Ok(youtube_result_to_playlist_items(api.video(&video_id).await)),
                    YoutubeUrlType::Short(short_id) => Ok(youtube_result_to_playlist_items(api.video(&short_id).await)),
                    YoutubeUrlType::None => Self::get_playlist(url.as_str(), YOUTUBE_DL_BACKEND::YT_DLP).await,
                }
            },
            PotPlayInputType::SpotifyUrl(url) => {
                match spotify_url_extractor(&url) {
                    _ => Self::get_playlist(url.as_str(), YOUTUBE_DL_BACKEND::YOUTUBE_DL).await,
                }
            },
            PotPlayInputType::Search(query) => {
                // Search way
                Self::get_playlist(&format!("ytsearch1:{}", query), YOUTUBE_DL_BACKEND::YT_DLP).await
            },
        };

        match playlist_result {
            Ok(mut new_playlist_items) => {
                let playlist_items_len = new_playlist_items.len();
                if playlist_items_len == 0 { return Err(anyhow!("No items in playlist")) }

                // Check if guilds playlists contains a playlist for the guild
                if !self.guilds_playlists.contains_key(guild_id) {
                    // Create a new empty list for guild
                    self.guilds_playlists.insert(*guild_id, Vec::new());
                }

                // Get a reference for the guild playlist
                let guild_playlist = self.guilds_playlists.get_mut(&guild_id).unwrap();

                if is_url {
                    // If the input type was an url we just append the new playlist items
                    guild_playlist.append(&mut new_playlist_items);

                    let index = guild_playlist.len() - playlist_items_len;
                    let slice = &guild_playlist[index..];
                    Ok((playlist_items_len, slice))
                } else {
                    // If the input was not an url we just get the first item and push it to the guild playlist
                    guild_playlist.push(new_playlist_items.remove(0));

                    let index = guild_playlist.len() - 1;
                    let slice = &guild_playlist[index..];
                    Ok((1, slice))
                }
            },
            Err(err) => Err(err),
        }
    }

    /// Remove all items from the playlist and returns true if the playlist is cleared of false if the guild has no playlist
    pub fn clear(&mut self, guild_id: &Id<GuildMarker>) -> bool{
        if self.guilds_playlists.contains_key(guild_id) { // Guild playlist already exist
            let guild_playlist = self.guilds_playlists.get_mut(guild_id).unwrap();
            guild_playlist.clear();

            true
        } else { // The guild playlist is not currently in the system
            false
        }
    }

    /// Fetch playlist with yt-dlp and parse the result
    async fn get_playlist (url: &str, backend: YOUTUBE_DL_BACKEND) -> anyhow::Result<Vec<PlaylistItem>> {
        let ytdl_args = [
            "-j",
            "-f",
            "webm[abr>0]/bestaudio/best",
            "-R",
            "infinite",
            "--yes-playlist",
            "--ignore-config",
            "--no-warnings",
            url,
            "-o",
            "-",
        ];

        let mut ytdlp_child = Command::new(backend.value())
            .args(ytdl_args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        // This rigmarole is required due to the inner synchronous reading context.
        let stderr = ytdlp_child.stderr.take();

        let (_returned_stderr, value) = task::spawn_blocking(move || {
            let mut child_stderr = stderr.unwrap();

            let mut output_data: String = String::new();
            let _ = child_stderr.read_to_string(&mut output_data);

            (child_stderr, output_data)
        })
        .await?;

        let _ = ytdlp_child.wait();

        let jsons: Vec<&str> = value.split('\n').collect();

        let items: Vec<PlaylistItem> = jsons.iter().filter_map(|json_str| {
            match serde_json::from_str::<PlaylistItem>(json_str) {
                Ok(mut item) => {
                    item.backend = Some(backend);
                    Some(item)
                },
                Err(_) => None,
            }
        }).collect();

        Ok(items)
    }

    pub async fn get_media (&self, item: &PlaylistItem) -> anyhow::Result<songbird::input::File<std::string::String>> {
        let _ = helpers::graceful_mkdir("data/cache");
        let fpath = format!("data/cache/media/{}/{}", item.extractor, item.id);
        let path = Path::new(&fpath);

        let file_path = if Self::check_file(path) {
            println!("Loaded from cache");
            Some(path.to_str().unwrap().to_string())
        } else {
            println!("Loaded from ytdl");
            let path_str = path.to_str().unwrap();

            Self::ytdlp_download(
                path_str, 
                &item.original_url, 
                *item.backend
                    .as_ref()
                    .unwrap_or(&YOUTUBE_DL_BACKEND::YT_DLP)
            ).await;
    
            if Self::check_file(path) {
                Some(path_str.to_string())
            } else {
                None
            }
        };

        match file_path {
            Some(file_path) => {
                let source = songbird::input::File::new(file_path);
                Ok(source)
            },
            None => Err(anyhow!("No file path")),
        }
    }

    // pub async fn get_media_stream(&self, item: &PlaylistItem) -> anyhow::Result<songbird::input::Input> {
    //     let ytdlp_child = Self::ytdlp_stream(
    //         &item.original_url,
    //         *item.backend
    //                 .as_ref()
    //                 .unwrap_or(&YOUTUBE_DL_BACKEND::YT_DLP)
    //     ).await?;
    //     let input = Self::ffmpeg_to_input(ytdlp_child).await?;
    //     Ok(input)
    // }

    pub async fn ytdlp_download(path_str: &str, item_original_url: &str, backend: YOUTUBE_DL_BACKEND) {
        let ytdl_args = [
            "--print-json",
            "-f",
            "webm[abr>0]/bestaudio/best",
            "-R",
            "infinite",
            "--no-playlist",
            "--ignore-config",
            "--no-warnings",
            item_original_url,
            "-o",
            path_str,
        ];

        println!("{ytdl_args:?}");

        let mut yt_dlp = Command::new(backend.value())
            .args(ytdl_args)
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .stdout(Stdio::null())
            .spawn().expect("yt-dlp failed to execute");

        let _ = yt_dlp.wait();
    }

    // Calls yt-dlp and gets the file data from stdout
    pub async fn ytdlp_stream(item_original_url: &str, backend: YOUTUBE_DL_BACKEND) -> anyhow::Result<std::process::Child> {
        let ytdl_args = [
            "--print-json",
            "-f",
            "webm[abr>0]/bestaudio/best",
            "-R",
            "infinite",
            "--no-playlist",
            "--ignore-config",
            "--no-warnings",
            item_original_url,
            "-o",
            "-",
        ];

        // let log = fs::File::create("debug.txt").expect("failed to open log");

        let mut yt_dlp = Command::new(backend.value())
            .args(ytdl_args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn().expect("ytdlp failed to execute");

        // This rigmarole is required due to the inner synchronous reading context.
        let stderr = yt_dlp.stderr.take();
        let returned_stderr = task::spawn_blocking(move || {
            let mut children_stderr = stderr.unwrap();

            let mut reader = BufReader::new(children_stderr.by_ref());

            let mut o_vec = vec![];
            let _ = reader.read_until(0xA, &mut o_vec);

            children_stderr
        })
        .await?;

        yt_dlp.stderr = Some(returned_stderr);

        Ok(yt_dlp)
    }

    // pub async fn ffmpeg_to_input(mut input: std::process::Child) -> anyhow::Result<songbird::input::Input>{
    //     let taken_stdout = input.stdout.take().ok_or_else(|| anyhow!("Failed to take children stdout"))?;

    //     let ffmpeg_args = [
    //         "-f",
    //         "s16le",
    //         "-ac",
    //         "2",
    //         "-ar",
    //         "48000",
    //         "-acodec",
    //         "pcm_f32le",
    //         "-",
    //     ];

    //     let ffmpeg = Command::new("ffmpeg")
    //         .arg("-i")
    //         .arg("-")
    //         .args(ffmpeg_args)
    //         .stdin(taken_stdout)
    //         .stderr(Stdio::inherit())
    //         .stdout(Stdio::piped())
    //         .spawn()?;

    //     Ok(songbird::input::Input::new(
    //         true,
    //         songbird::input::children_to_reader::<f32>(vec![input, ffmpeg]),
    //         songbird::input::Codec::FloatPcm,
    //         songbird::input::Container::Raw,
    //         Default::default(),
    //     ))
    // }

    pub async fn save_stdout(input: ChildStdout) -> anyhow::Result<()>{
        let tee_args = [
            "debug.txt",
        ];

        let mut _tee = Command::new("tee")
            .args(tee_args)
            .stdin(input)
            .stderr(Stdio::inherit())
            .stdout(Stdio::inherit())
            .spawn()?;
        
            // let _ = _tee.wait();

        Ok(())
    }

    fn check_file (path: &Path) -> bool {
        match fs::metadata(path) {
            Ok(attributes) => {
                !attributes.is_dir()
            },
            Err(_) => {
                false
            }
        }
    }
}

impl Default for SystemPlaylist {
    fn default() -> Self {
        Self::new()
    }
}

    
#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct PlaylistItem {
    pub id: String,
    pub title: String,
    pub original_url: String,
    pub extractor: String,
    pub thumbnail: Option<String>,
    pub duration: Option<f32>,
    pub playlist_id: Option<String>,
    pub webpage_url: Option<String>,
    pub is_live: Option<bool>,
    pub was_live: Option<bool>,
    pub backend: Option<YOUTUBE_DL_BACKEND>
}