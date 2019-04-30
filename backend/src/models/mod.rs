use chrono::NaiveDateTime;
use serde::Serialize;

#[derive(Serialize)]
pub struct User {
    pub id: i64,
    pub creation_time: NaiveDateTime,
    pub last_update_time: NaiveDateTime,
    pub username: String,
    pub token: String,
    pub refresh_token: String,
}

#[derive(Serialize)]
pub struct Track {
    pub id: i64,
    pub title: String,
    pub artists: String,
    pub preview_url: String,
    pub album: String,
    pub image_url: String,
}

#[derive(Serialize)]
pub struct Artist {
    pub id: i64,
    pub name: String,
    pub genres: String,
    pub image_url: String,
    pub uri: String,
}

#[derive(Serialize)]
pub struct TimeFrames<T: Serialize> {
    pub short: Vec<T>,
    pub medium: Vec<T>,
    pub long: Vec<T>,
}

#[derive(Serialize)]
pub struct StatsSnapshot {
    pub last_update_time: NaiveDateTime,
    pub tracks: TimeFrames<Track>,
    pub artists: TimeFrames<Artist>,
}
