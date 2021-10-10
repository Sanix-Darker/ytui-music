use crate::{Fetcher, ReturnAction};
use reqwest;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.131 Safari/537.36";
const FIELDS: [&str; 3] = [
    "fields=videoId,title,author,lengthSeconds",
    "fields=title,playlistId,author,videoCount",
    "fields=author,authorId,videoCount",
];
pub const ITEM_PER_PAGE: usize = 10;
const REGION: &str = "region=NP";
const FILTER_TYPE: [&str; 3] = ["music", "playlist", "channel"];
const REQUEST_PER_SERVER: u8 = 10;

impl crate::ExtendDuration for Duration {
    fn to_string(self) -> String {
        let seconds: u64 = self.as_secs();
        let mut res = format!(
            "{minutes}:{seconds}",
            minutes = seconds / 60,
            seconds = seconds % 60
        );
        res.shrink_to_fit();
        res
    }

    // This function assumes that the string is alwayd formatted in "min:secs"
    fn from_string(inp: &str) -> Duration {
        let splitted = inp.split_once(':').unwrap();
        let total_secs: u64 = (60 * splitted.0.trim().parse::<u64>().unwrap_or_default())
            + splitted.1.trim().parse::<u64>().unwrap_or_default();
        Duration::from_secs(total_secs)
    }
}

impl Fetcher {
    pub fn new(server_list: &'static [String]) -> Self {
        super::Fetcher {
            trending_now: None,
            playlist_content: super::PlaylistRes::default(),
            artist_content: super::ArtistRes::default(),
            search_res: super::SearchRes::default(),
            servers: server_list,
            client: reqwest::ClientBuilder::default()
                .user_agent(USER_AGENT)
                .gzip(true)
                .build()
                .unwrap(),
            active_server_index: 0,
            request_sent: 0,
        }
    }
    pub fn change_server(&mut self) {
        self.active_server_index = (self.active_server_index + 1) % self.servers.len();
    }
}

macro_rules! search {
    ("music", $fetcher: expr, $query: expr, $page: expr) => {
        search!(
            "@internal-core",
            $fetcher,
            $query,
            $page,
            $fetcher.search_res.music,
            0,
            super::MusicUnit
        )
    };
    ("playlist", $fetcher: expr, $query: expr, $page: expr) => {
        search!(
            "@internal-core",
            $fetcher,
            $query,
            $page,
            $fetcher.search_res.playlist,
            1,
            super::PlaylistUnit
        )
    };
    ("artist", $fetcher: expr, $query: expr, $page: expr) => {
        search!(
            "@internal-core",
            $fetcher,
            $query,
            $page,
            $fetcher.search_res.artist,
            2,
            super::ArtistUnit
        )
    };

    ("@internal-core", $fetcher: expr, $query: expr, $page: expr, $store_target: expr, $filter_index: expr, $unit_type: ty) => {{
        let suffix = format!(
            "/search?q={query}&type={s_type}&{region}&page={page}&{fields}",
            query = $query,
            s_type = FILTER_TYPE[$filter_index],
            region = REGION,
            fields = FIELDS[$filter_index],
            page = $page
        );
        let lower_limit = $page * ITEM_PER_PAGE;
        let mut upper_limit = std::cmp::min($store_target.len(), lower_limit + ITEM_PER_PAGE);

        let is_new_query = *$query != $fetcher.search_res.query;
        let is_new_type = $fetcher.search_res.last_fetched != $filter_index;
        let insufficient_data = upper_limit.checked_sub(lower_limit).unwrap_or(0) < ITEM_PER_PAGE;

        $fetcher.search_res.last_fetched = $filter_index;
        if is_new_query || insufficient_data || is_new_type {
            let obj = $fetcher.send_request::<Vec<$unit_type>>(&suffix, 1).await;
            if is_new_query || is_new_type {
                $store_target.clear();
            }
            match obj {
                Ok(data) => {
                    $fetcher.search_res.query = $query.to_string();
                    $store_target.extend_from_slice(data.as_slice());
                    upper_limit = std::cmp::min($store_target.len(), lower_limit + ITEM_PER_PAGE);
                }
                Err(e) => return Err(e),
            }
        }

        if upper_limit > lower_limit {
            Ok($store_target[lower_limit..upper_limit].to_vec())
        } else {
            Err(ReturnAction::EOR)
        }
    }};
}

impl Fetcher {
    // All the request should be send from this function
    async fn send_request<'de, Res>(
        &mut self,
        path: &str,
        retry_for: i32,
    ) -> Result<Res, ReturnAction>
    where
        Res: serde::de::DeserializeOwned,
    {
        let res = self
            .client
            .get(self.servers[self.active_server_index].to_string() + path)
            .send()
            .await;

        // Change server time to time.
        if self.request_sent > REQUEST_PER_SERVER {
            self.change_server();
        }
        self.request_sent += 1;

        match res {
            Ok(response) => {
                if let Ok(obj) = response.json::<Res>().await {
                    Ok(obj)
                } else {
                    Err(ReturnAction::Failed)
                }
            }
            Err(_) if retry_for > 0 => {
                self.change_server();
                Err(ReturnAction::Retry)
            }
            Err(_) => Err(ReturnAction::Failed),
        }
    }

    pub async fn get_trending_music(
        &mut self,
        page: usize,
    ) -> Result<Vec<super::MusicUnit>, ReturnAction> {
        let lower_limit = ITEM_PER_PAGE * page;

        if self.trending_now.is_none() {
            let suffix = format!(
                "/trending?type=Music&{region}&{music_field}",
                region = REGION,
                music_field = FIELDS[0]
            );

            let obj = self.send_request::<Vec<super::MusicUnit>>(&suffix, 2).await;
            match obj {
                Ok(mut res) => {
                    res.shrink_to_fit();
                    self.trending_now = Some(res);
                }
                Err(e) => return Err(e),
            }
        }

        let trending_now = self.trending_now.as_ref().unwrap();
        let upper_limit = std::cmp::min(trending_now.len(), lower_limit + ITEM_PER_PAGE);

        if lower_limit >= upper_limit {
            Err(ReturnAction::EOR)
        } else {
            Ok(trending_now[lower_limit..upper_limit].to_vec())
        }
    }

    pub async fn get_playlist_content(
        &mut self,
        playlist_id: &str,
        page: usize,
    ) -> Result<Vec<super::MusicUnit>, ReturnAction> {
        let lower_limit = page * ITEM_PER_PAGE;

        let is_new_id = *playlist_id != self.playlist_content.id;
        if is_new_id {
            self.playlist_content.id = playlist_id.to_string();
            let suffix = format!(
                "/playlists/{playlist_id}?fields=videos",
                playlist_id = playlist_id
            );

            let obj = self
                .send_request::<super::FetchPlaylistContentRes>(&suffix, 1)
                .await;
            match obj {
                Ok(mut data) => {
                    data.videos.shrink_to_fit();
                    self.playlist_content.music = data.videos;
                }
                Err(e) => return Err(e),
            }
        }

        let upper_limit = std::cmp::min(
            self.playlist_content.music.len(),
            lower_limit + ITEM_PER_PAGE,
        );
        if lower_limit >= upper_limit {
            Err(ReturnAction::EOR)
        } else {
            let mut res = self.playlist_content.music[lower_limit..upper_limit].to_vec();
            res.shrink_to_fit();
            Ok(res)
        }
    }

    pub async fn get_playlist_of_channel(
        &mut self,
        channel_id: &str,
        page: usize,
    ) -> Result<Vec<super::PlaylistUnit>, ReturnAction> {
        let lower_limit = page * ITEM_PER_PAGE;

        let is_new_id = *channel_id != self.artist_content.playlist.0;
        if is_new_id || self.artist_content.playlist.1.is_empty() {
            self.artist_content.playlist.0 = channel_id.to_string();
            let suffix = format!(
                "/channels/{channel_id}/playlists?fields=playlists",
                channel_id = channel_id
            );

            let obj = self
                .send_request::<super::FetchArtistPlaylist>(&suffix, 1)
                .await;
            match obj {
                Ok(mut data) => {
                    data.playlists.shrink_to_fit();
                    self.artist_content.playlist.1 = data.playlists;
                }
                Err(e) => return Err(e),
            }
        }

        let upper_limit = std::cmp::min(
            self.artist_content.playlist.1.len(),
            lower_limit + ITEM_PER_PAGE,
        );
        if lower_limit >= upper_limit {
            Err(ReturnAction::EOR)
        } else {
            let mut res = self.artist_content.playlist.1[lower_limit..upper_limit].to_vec();
            res.shrink_to_fit();
            Ok(res)
        }
    }

    pub async fn get_videos_of_channel(
        &mut self,
        channel_id: &str,
        page: usize,
    ) -> Result<Vec<super::MusicUnit>, ReturnAction> {
        let lower_limit = page * ITEM_PER_PAGE;

        let is_new_id = *channel_id != self.artist_content.music.0;
        if is_new_id || self.artist_content.music.1.is_empty() {
            self.artist_content.music.0 = channel_id.to_string();
            let suffix = format!("/channels/{channel_id}/videos", channel_id = channel_id);

            let obj = self.send_request::<Vec<super::MusicUnit>>(&suffix, 1).await;
            match obj {
                Ok(mut data) => {
                    data.shrink_to_fit();
                    self.artist_content.music.1 = data;
                }
                Err(e) => return Err(e),
            }
        }

        let upper_limit = std::cmp::min(
            self.artist_content.music.1.len(),
            lower_limit + ITEM_PER_PAGE,
        );
        if lower_limit >= upper_limit {
            Err(ReturnAction::EOR)
        } else {
            let mut res = self.artist_content.music.1[lower_limit..upper_limit].to_vec();
            res.shrink_to_fit();
            Ok(res)
        }
    }

    pub async fn search_music(
        &mut self,
        query: &str,
        page: usize,
    ) -> Result<Vec<super::MusicUnit>, ReturnAction> {
        search!("music", self, query, page)
    }

    pub async fn search_playlist(
        &mut self,
        query: &str,
        page: usize,
    ) -> Result<Vec<super::PlaylistUnit>, ReturnAction> {
        search!("playlist", self, query, page)
    }

    pub async fn search_artist(
        &mut self,
        query: &str,
        page: usize,
    ) -> Result<Vec<super::ArtistUnit>, ReturnAction> {
        search!("artist", self, query, page)
    }
}

// ------------- TEST ----------------
#[cfg(test)]
mod tests {
    use super::*;
    use lazy_static::lazy_static;

    lazy_static! {
        static ref FETCHER: Vec<String> = vec![String::from("https://ytprivate.com/api/v1/")];
    }
    fn get_fetcher_for_test() -> Fetcher {
        // FIXME: how can I get a static variable for test
        Fetcher::new(&FETCHER)
    }

    #[tokio::test]
    async fn test_trending_extractor() {
        let mut fetcher = get_fetcher_for_test();
        let mut page = 0;

        while let Ok(data) = fetcher.get_trending_music(page).await {
            println!("--------- Trending [{}] ----------", page);
            println!("{:#?}", data);
            page += 1;
        }
    }

    #[tokio::test]
    async fn check_format() {
        let sample_response = r#"{
                                    "title": "Some song title",
                                    "videoId": "WNgO6G7uERU",
                                    "author": "CHHEWANG",
                                    "lengthSeconds": 271
                                }"#;
        let obj: super::super::MusicUnit = serde_json::from_str(sample_response).unwrap();
        assert_eq!(
            obj,
            super::super::MusicUnit {
                artist: "CHHEWANG".to_string(),
                name: "Some song title".to_string(),
                duration: "4:31".to_string(),
                id: "WNgO6G7uERU".to_string(),
            },
        );
    }
    #[tokio::test]
    async fn check_music_search() {
        let mut fetcher = get_fetcher_for_test();
        let obj = fetcher.search_music("Bartika Eam Rai", 0).await;
        eprintln!("{:#?}", obj);
    }

    #[tokio::test]
    async fn check_playlist_search() {
        let mut fetcher = get_fetcher_for_test();
        let obj = fetcher.search_playlist("Spotify Chill mix", 0).await;
        eprintln!("{:#?}", obj);
    }

    #[tokio::test]
    async fn check_artist_search() {
        let mut fetcher = get_fetcher_for_test();
        let obj = fetcher.search_artist("Rachana Dahal", 0).await;
        eprintln!("{:#?}", obj);
    }

    #[tokio::test]
    async fn check_playlist_content() {
        let mut fetcher = get_fetcher_for_test();
        let obj = fetcher
            .get_playlist_content("PLN4UKncphTTE0cIHYDQy534mSToKtFlhA", 0)
            .await;
        eprintln!("{:#?}", obj);
    }

    #[tokio::test]
    async fn check_artist_content_music() {
        let mut fetcher = get_fetcher_for_test();
        for channel_id in ["UCJEog_sDzuGyLok8f4p0HRA", "UCIwFjwMjI0y7PDBVEO9-bkQ"] {
            let obj = fetcher.get_videos_of_channel(channel_id, 0).await;
            eprintln!("{:#?}", obj);
        }
    }

    #[tokio::test]
    async fn check_artist_content_playlist() {
        let mut fetcher = get_fetcher_for_test();
        let obj = fetcher
            .get_playlist_of_channel("UCJEog_sDzuGyLok8f4p0HRA", 0)
            .await;
        eprintln!("{:#?}", obj);
    }
}
