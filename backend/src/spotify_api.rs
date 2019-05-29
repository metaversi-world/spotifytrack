use std::collections::HashMap;
use std::thread;

use chrono::Utc;
use crossbeam::channel;
use diesel::prelude::*;
use reqwest;
use serde::{Deserialize, Serialize};

use crate::conf::CONF;
use crate::models::{
    AccessTokenResponse, Artist, NewArtistHistoryEntry, NewTrackHistoryEntry,
    SpotifyBatchArtistsResponse, SpotifyBatchTracksResponse, StatsSnapshot, TopArtistsResponse,
    TopTracksResponse, Track, User, UserProfile,
};
use crate::DbConn;

const SPOTIFY_USER_RECENTLY_PLAYED_URL: &str =
    "https://api.spotify.com/v1/me/player/recently-played";
const SPOTIFY_USER_PROFILE_INFO_URL: &str = "https://api.spotify.com/v1/me";
const SPOTIFY_BATCH_TRACKS_URL: &str = "https://api.spotify.com/v1/tracks";
const SPOTIFY_BATCH_ARTISTS_URL: &str = "https://api.spotify.com/v1/artists";
const SPOTIFY_APP_TOKEN_URL: &str = "https://accounts.spotify.com/api/token";
const ENTITY_FETCH_COUNT: usize = 50;

fn get_top_entities_url(entity_type: &str, timeframe: &str) -> String {
    format!(
        "https://api.spotify.com/v1/me/top/{}?limit={}&time_range={}_term",
        entity_type, ENTITY_FETCH_COUNT, timeframe
    )
}

pub fn get_user_profile_info(token: &str) -> Result<UserProfile, String> {
    let client = reqwest::Client::new();
    let mut res = client
        .get(SPOTIFY_USER_PROFILE_INFO_URL)
        .bearer_auth(token)
        .send()
        .map_err(|err| -> String {
            error!("Error fetching user profile from Spotify API: {:?}", err);
            "Error requesting latest user profile info from the Spotify API".into()
        })?;

    res.json().map_err(|err| -> String {
        error!(
            "Error parsing user profile info response from Spotify API: {:?}",
            err
        );
        "Error parsing user profile infor response from Spotify API".into()
    })
}

pub fn fetch_auth_token() -> Result<String, String> {
    let client = reqwest::Client::new();
    let mut params = HashMap::new();
    params.insert("grant_type", "client_credentials");

    let mut res = client
        .post(SPOTIFY_APP_TOKEN_URL)
        .header(
            "Authorization",
            format!(
                "Basic {}",
                base64::encode(&format!("{}:{}", CONF.client_id, CONF.client_secret))
            ),
        )
        .form(&params)
        .send()
        .map_err(|err| -> String {
            error!("Error fetching token from Spotify API: {:?}", err);
            "Error requesting access token from the Spotify API".into()
        })?;

    res.json::<AccessTokenResponse>()
        .map_err(|err| {
            error!(
                "Error decoding response while fetching token from Spotify API: {:?}",
                err
            );
            "Error decoding response from Spotify API".into()
        })
        .map(|res| res.access_token)
}

pub fn fetch_cur_stats(user: &User) -> Result<Option<StatsSnapshot>, String> {
    // Use the user's token to fetch their current stats
    let (tx, rx) = channel::unbounded::<(
        &'static str,
        &'static str,
        Result<reqwest::Response, String>,
    )>();

    // Create threads for each of the inner requests (we have to make 6; one for each of the three
    // timeframes, and then that multiplied by each of the two entities (tracks and artists)).
    debug!("Kicking off 6 API requests on separate threads...");
    for entity_type in &["tracks", "artists"] {
        for timeframe in &["short", "medium", "long"] {
            let token = user.token.clone();
            let tx = tx.clone();

            thread::spawn(move || {
                let client = reqwest::Client::new();
                let res: Result<reqwest::Response, String> = client
                    .get(&get_top_entities_url(entity_type, timeframe))
                    .bearer_auth(token)
                    .send()
                    .map_err(|_err| -> String {
                        "Error requesting latest user stats from the Spotify API".into()
                    });

                tx.send((entity_type, timeframe, res))
            });
        }
    }

    let mut stats_snapshot = StatsSnapshot::new(Utc::now().naive_utc());

    // Wait for all 6 requests to return back and then
    debug!("Waiting for all 6 inner stats requests to return...");
    for _ in 0..6 {
        match rx.recv().unwrap() {
            ("tracks", timeframe, res) => {
                let parsed_res: TopTracksResponse = res?.json().map_err(|err| -> String {
                    error!("Error parsing top tracks response: {:?}", err);
                    "Error parsing response from Spotify".into()
                })?;

                for top_track in parsed_res.items.into_iter() {
                    stats_snapshot.tracks.add_item(timeframe, top_track);
                }
            }
            ("artists", timeframe, res) => {
                let parsed_res: TopArtistsResponse = res?.json().map_err(|err| -> String {
                    error!("Error parsing top artists response: {:?}", err);
                    "Error parsing response from Spotify".into()
                })?;

                for top_artist in parsed_res.items.into_iter() {
                    stats_snapshot.artists.add_item(timeframe, top_artist);
                }
            }
            _ => unreachable!(),
        }
    }

    Ok(Some(stats_snapshot))
}

fn map_timeframe_to_timeframe_id(timeframe: &str) -> u8 {
    match timeframe {
        "short" => 0,
        "medium" => 1,
        "long" => 2,
        _ => panic!(format!(
            "Tried to convert invalid timeframe to id: \"{}\"",
            timeframe
        )),
    }
}

/// For each track and artist timeframe, store a row in the `track_history` and `artist_history`
/// tables respectively
pub fn store_stats_snapshot(conn: DbConn, user: &User, stats: StatsSnapshot) -> Result<(), String> {
    use crate::schema::users::dsl::*;

    let update_time = stats.last_update_time;

    let artist_entries: Vec<NewArtistHistoryEntry> = stats
        .artists
        .into_iter()
        .flat_map(|(artist_timeframe, artists)| {
            artists
                .into_iter()
                .enumerate()
                .map(move |(artist_ranking, artist)| NewArtistHistoryEntry {
                    user_id: user.id,
                    spotify_id: artist.id,
                    update_time,
                    timeframe: map_timeframe_to_timeframe_id(&artist_timeframe),
                    ranking: artist_ranking as u16,
                })
        })
        .collect();

    diesel::insert_into(crate::schema::artist_history::table)
        .values(&artist_entries)
        .execute(&conn.0)
        .map_err(|err| -> String {
            println!("Error inserting row: {:?}", err);
            "Error inserting user into database".into()
        })?;

    let track_entries: Vec<NewTrackHistoryEntry> = stats
        .tracks
        .into_iter()
        .flat_map(|(track_timeframe, tracks)| {
            tracks
                .into_iter()
                .enumerate()
                .map(move |(track_ranking, track)| NewTrackHistoryEntry {
                    user_id: user.id,
                    spotify_id: track.id,
                    update_time,
                    timeframe: map_timeframe_to_timeframe_id(&track_timeframe),
                    ranking: track_ranking as u16,
                })
        })
        .collect();

    diesel::insert_into(crate::schema::track_history::table)
        .values(&track_entries)
        .execute(&conn.0)
        .map_err(|err| -> String {
            println!("Error inserting row: {:?}", err);
            "Error inserting user into database".into()
        })?;

    // Update the user to have a last update time that matches all of the new updates
    let updated_row_count = diesel::update(users.filter(id.eq(user.id)))
        .set(last_update_time.eq(update_time))
        .execute(&conn.0)
        .map_err(|err| -> String {
            error!("Error updating user's last update time: {:?}", err);
            "Error updating user's last update time.".into()
        })?;

    if updated_row_count != 1 {
        error!(
            "Updated {} rows when setting last update time, but should have updated 1.",
            updated_row_count
        );
    }

    Ok(())
}

const MAX_BATCH_ENTITY_COUNT: usize = 50;

fn fetch_batch_entities<T: for<'de> Deserialize<'de>>(
    base_url: &str,
    spotify_entity_ids: &[&str],
) -> Result<T, String> {
    let url = format!("{}?ids={}", base_url, spotify_entity_ids.join(","));
    let client = reqwest::Client::new();
    client
        .get(&url)
        .bearer_auth(&fetch_auth_token()?) // TODO: get this from managed state
        .send()
        .map_err(|_err| -> String { "Error requesting batch data from the Spotify API".into() })?
        .json()
        .map_err(|err| -> String {
            error!("Error decoding JSON from Spotify API: {:?}", err);
            "Error reading data from the Spotify API".into()
        })
}

// TODO: Enforce API page size limitations and recursively call (or whatever) until all is fetched
fn fetch_with_cache<
    ResponseType: for<'de> Deserialize<'de>,
    T: Clone + Serialize + for<'de> Deserialize<'de>,
>(
    cache_key: &str,
    api_url: &str,
    spotify_ids: &[&str],
    map_response_to_items: fn(ResponseType) -> Result<Vec<T>, String>,
) -> Result<Vec<T>, String> {
    // First, try to get as many items as we can from the cache
    let cache_res = crate::cache::get_hash_items::<T>(cache_key, spotify_ids)?;

    // Fire off a request to Spotify to fill in the missing items
    let mut missing_indices = Vec::new();
    let mut missing_ids = Vec::new();
    for (i, datum) in cache_res.iter().enumerate() {
        if datum.is_none() {
            missing_indices.push(i);
            missing_ids.push(spotify_ids[i]);
        }
    }

    let mut fetched_entities = Vec::with_capacity(missing_indices.len());
    for (chunk_ix, chunk) in missing_ids.chunks(MAX_BATCH_ENTITY_COUNT).enumerate() {
        let res: ResponseType = fetch_batch_entities(api_url, chunk)?;
        let fetched_artist_data = map_response_to_items(res)?;

        // Update the cache with the missing items
        crate::cache::set_hash_items(
            cache_key,
            &fetched_artist_data
                .iter()
                .enumerate()
                .map(|(i, datum)| (missing_ids[(chunk_ix * MAX_BATCH_ENTITY_COUNT) + i], datum))
                .collect::<Vec<_>>(),
        )?;

        fetched_entities.extend(fetched_artist_data)
    }

    let mut i = 0;
    let combined_results = cache_res
        .into_iter()
        .map(|opt| {
            opt.unwrap_or_else(|| {
                // We could avoid this clone by reversing the direction in which we fetch the items
                // but that's 100% premature and likely useless optimization
                let val = fetched_entities[i].clone();
                i += 1;
                val
            })
        })
        .collect::<Vec<_>>();
    Ok(combined_results)
}

pub fn fetch_artists(spotify_ids: &[&str]) -> Result<Vec<Artist>, String> {
    fetch_with_cache::<SpotifyBatchArtistsResponse, _>(
        &crate::conf::CONF.artists_cache_hash_name,
        SPOTIFY_BATCH_ARTISTS_URL,
        spotify_ids,
        |res: SpotifyBatchArtistsResponse| Ok(res.artists),
    )
}

pub fn fetch_tracks(spotify_ids: &[&str]) -> Result<Vec<Track>, String> {
    fetch_with_cache::<SpotifyBatchTracksResponse, _>(
        &crate::conf::CONF.tracks_cache_hash_name,
        SPOTIFY_BATCH_TRACKS_URL,
        spotify_ids,
        |res: SpotifyBatchTracksResponse| Ok(res.tracks),
    )
}
