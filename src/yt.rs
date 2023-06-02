use std::collections::HashMap;

use serde::Deserialize;
use async_recursion::async_recursion;

pub struct YoutubeAPI {
    key: String
}

impl YoutubeAPI {
    pub fn new (key: &str) -> Self {
        Self {
            key: key.to_owned()
        }
    }

    pub async fn video (&self, id: &str) -> YoutubeResult {
        let search_url = format!("https://www.googleapis.com/youtube/v3/videos?key={}&part=snippet&maxResults=1&id={}", &self.key, id);
        let result = reqwest::get(search_url).await;

        match result {
            Ok(response) => {
                match response.text().await {
                    Ok(text) => {
                        // println!("{}", text);
                        let playlist_items_result = serde_json::from_str::<YoutubePlaylistItemsResponse>(&text);

                        match playlist_items_result {
                            Ok(mut result) => {
                                for item in result.items.iter_mut() {
                                    item.snippet.resourceId = Some(YoutubeItemID {
                                        kind: item.kind.to_owned(),
                                        videoId: item.id.to_owned(),
                                    })
                                }
                                YoutubeResult::Ok(result)
                            },
                            Err(_) => {
                                let error_result = serde_json::from_str::<YoutubeErrorResponse>(&text);

                                match error_result {
                                    Ok(error) => YoutubeResult::Error(error.error),
                                    Err(_) => YoutubeResult::UnknownError(text),
                                }
                            },
                        }
                    },
                    Err(_) => YoutubeResult::TextExtractionError,
                }
            },
            Err(_) => YoutubeResult::RequestError,
        }
    }

    pub async fn _search (&self, query: &str) -> YoutubeResult {
        let search_url = format!("https://www.googleapis.com/youtube/v3/search?key={}&part=snippet&maxResults=1&type=video&q={}", &self.key, query);
        let result = reqwest::get(search_url).await;

        match result {
            Ok(response) => {
                match response.text().await {
                    Ok(text) => {
                        let playlist_items_result = serde_json::from_str::<YoutubeSearchResponse>(&text);

                        match playlist_items_result {
                            Ok(result) => YoutubeResult::Ok(result._to_playlist_response()),
                            Err(_) => {
                                let error_result = serde_json::from_str::<YoutubeErrorResponse>(&text);

                                match error_result {
                                    Ok(error) => YoutubeResult::Error(error.error),
                                    Err(_) => YoutubeResult::UnknownError(text),
                                }
                            },
                        }
                    },
                    Err(_) => YoutubeResult::TextExtractionError,
                }
            },
            Err(_) => YoutubeResult::RequestError,
        }
    }

    pub async fn playlist (&self, playlist: &str) -> YoutubeResult {
        let search_url = format!("https://www.googleapis.com/youtube/v3/playlistItems?key={}&part=snippet&maxResults=50&playlistId={}", &self.key, playlist);
        let result = reqwest::get(search_url).await;

        match result {
            Ok(response) => {
                match response.text().await {
                    Ok(text) => {
                        let playlist_items_result = serde_json::from_str::<YoutubePlaylistItemsResponse>(&text);

                        match playlist_items_result {
                            Ok(mut result) => {
                                if let Some(next_page_token) = &result.nextPageToken {
                                    result.items.append(&mut self.playlist_get_items (playlist, Some(next_page_token)).await);
                                    YoutubeResult::Ok(result)
                                } else {
                                    YoutubeResult::Ok(result)
                                }
                            },
                            Err(_) => {
                                let error_result = serde_json::from_str::<YoutubeErrorResponse>(&text);

                                match error_result {
                                    Ok(error) => YoutubeResult::Error(error.error),
                                    Err(_) => YoutubeResult::UnknownError(text),
                                }
                            },
                        }
                    },
                    Err(_) => YoutubeResult::TextExtractionError,
                }
            },
            Err(_) => YoutubeResult::RequestError,
        }
    }

    #[async_recursion]
    async fn playlist_get_items (&self, playlist: &str, page_token: Option<&'async_recursion str>) -> Vec<YoutubePlaylistItemsResult> {
        let search_url = if let Some(page_token_str) = &page_token {
            format!("https://www.googleapis.com/youtube/v3/playlistItems?key={}&part=snippet&maxResults=50&pageToken={}&playlistId={}", &self.key, page_token_str, playlist)
        } else {
            format!("https://www.googleapis.com/youtube/v3/playlistItems?key={}&part=snippet&maxResults=50&playlistId={}", &self.key, playlist)
        };

        let result = reqwest::get(search_url).await;

        match result {
            Ok(response) => {
                match response.text().await {
                    Ok(text) => {
                        let playlist_items_result = serde_json::from_str::<YoutubePlaylistItemsResponse>(&text);

                        match playlist_items_result {
                            Ok(mut result) => {
                                if let Some(next_page_token) = result.nextPageToken {
                                    result.items.append(&mut self.playlist_get_items (playlist, Some(&next_page_token)).await);
                                    result.items
                                } else {
                                    result.items
                                }
                            },
                            Err(_) => {
                                let error_result = serde_json::from_str::<YoutubeErrorResponse>(&text);

                                match error_result {
                                    Ok(_) => Vec::new(),
                                    Err(_) => Vec::new(),
                                }
                            },
                        }
                    },
                    Err(_) => Vec::new(),
                }
            },
            Err(_) => Vec::new(),
        }
    }
}

#[derive(Debug)]
pub enum YoutubeResult {
    Ok(YoutubePlaylistItemsResponse),
    Error(YoutubeError),
    RequestError,
    TextExtractionError,
    UnknownError(String)
}

// Error

#[derive(Deserialize, Debug)]
struct YoutubeErrorResponse {
    error: YoutubeError
}

#[allow(dead_code)]
#[derive(Deserialize, Debug)]
pub struct YoutubeError {
    code: i64,
    message: String
}

// Search

#[derive(Deserialize, Debug)]
pub struct YoutubeSearchResponse {
    pub kind: String,
    pub items: Vec<YoutubeSearchResult>
}

impl YoutubeSearchResponse {
    pub fn _to_playlist_response (&self) -> YoutubePlaylistItemsResponse {
        YoutubePlaylistItemsResponse {
            kind: self.kind.to_owned(),
            items: self.items.iter().map(|item| YoutubePlaylistItemsResult {
                kind: item.kind.to_owned(),
                id: item.id.videoId.to_owned(),
                snippet: YoutubeItemSnippet {
                    title: item.snippet.title.to_owned(),
                    resourceId: Some(YoutubeItemID {
                        kind: item.id.kind.to_owned(),
                        videoId: item.id.videoId.to_owned()
                    }),
                    thumbnails: item.snippet.thumbnails.clone()
                },
            }).collect(),
            nextPageToken: None,
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct YoutubeSearchResult {
    pub kind: String,
    pub id: YoutubeItemID,
    pub snippet: YoutubeItemSnippet
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct YoutubeItemID {
    pub kind: String,
    pub videoId: String
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct YoutubeItemSnippet {
    pub title: String,
    pub resourceId: Option<YoutubeItemID>,
    pub thumbnails: HashMap<String, YoutubeItemThumbnail>
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug, Clone)]
pub struct YoutubeItemThumbnail {
    pub url: String,
    pub width: u32,
    pub height: u32
}

// Playlist items

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
pub struct YoutubePlaylistItemsResponse {
    pub kind: String,
    pub items: Vec<YoutubePlaylistItemsResult>,
    pub nextPageToken: Option<String>
}

#[derive(Deserialize, Debug)]
pub struct YoutubePlaylistItemsResult {
    pub kind: String,
    pub id: String,
    pub snippet: YoutubeItemSnippet
}