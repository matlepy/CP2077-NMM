use std::time::Duration;

use reqwest::Client;
use serde::{Deserialize, Serialize};
use tokio::time::sleep;

use crate::errors::{AppError, AppResult};

/// Information about a single downloadable file attached to a Nexus mod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModFile {
    pub id: i32,
    pub name: String,
    pub size: u64,
    pub version: String,
    pub category: String,
    pub mod_id: i32,
    pub file_path: String,
    pub file_name: String,
    pub file_version: String,
    pub file_category: String,
    pub file_url: String,
}

/// Static mod metadata. Stored in the `mods` table; `required_mod_ids` /
/// `required_versions` are persisted via the side table `mod_requirements`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Mod {
    /// Internal DB primary key (0 if not yet inserted).
    #[serde(default)]
    pub id: i32,
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    pub nexus_id: String,
    #[serde(default)]
    pub required_mod_ids: Vec<String>,
    #[serde(default)]
    pub required_versions: std::collections::HashMap<String, String>,
}

/// Parsed parts of an `nxm://` URI. See Phase 3.5.
#[derive(Debug, Clone)]
pub struct NxmLink {
    pub game_domain: String,
    pub mod_id: i32,
    pub file_id: i32,
    pub key: Option<String>,
    pub expires: Option<i64>,
}

/// Nexus REST API rate-limit state. See Phase 3.4.
#[derive(Debug, Clone, Copy, Default)]
pub struct RateLimit {
    pub hourly_remaining: Option<u32>,
    pub daily_remaining: Option<u32>,
    pub hourly_limit: Option<u32>,
    pub daily_limit: Option<u32>,
}

/// REST client for `api.nexusmods.com/v1`. Thread-safe and cheap to clone.
#[derive(Debug, Clone)]
pub struct NexusClient {
    client: Client,
    api_key: String,
    base_url: String,
    game_id: i32,
    rate_limit: std::sync::Arc<std::sync::Mutex<RateLimit>>,
}

impl NexusClient {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("nexus-cp2077-mod-manager/0.1")
            .build()
            .expect("reqwest client build");
        Self {
            client,
            api_key,
            base_url: "https://api.nexusmods.com/v1".to_string(),
            game_id: 3333, // Cyberpunk 2077
            rate_limit: std::sync::Arc::new(std::sync::Mutex::new(RateLimit::default())),
        }
    }

    /// Set the game id used by search/details endpoints. Cyberpunk 2077 = 3333.
    pub fn with_game_id(mut self, game_id: i32) -> Self {
        self.game_id = game_id;
        self
    }

    /// Snapshot the most recent rate-limit headers.
    pub fn rate_limit(&self) -> RateLimit {
        *self.rate_limit.lock().expect("rate_limit mutex")
    }

    /// Validate the API key with `/v1/users/validate.json`. Phase 3.3.
    pub async fn validate_api_key(&self) -> AppResult<bool> {
        let response = self
            .request(reqwest::Method::GET, "/users/validate.json", &[])
            .await?;
        self.update_rate_limit(&response);
        if response.status().is_success() {
            Ok(true)
        } else {
            Err(AppError::Api(format!(
                "API key validation failed: {}",
                response.status()
            )))
        }
    }

    /// Search mods for the configured game. Phase 3.1.
    pub async fn search_mods(&self, query: &str) -> AppResult<Vec<Mod>> {
        let encoded = urlencoding(query);
        let path = format!("/games/{}/mods/search.json?term={encoded}", self.game_id);
        let response = self.request(reqwest::Method::GET, &path, &[]).await?;
        self.update_rate_limit(&response);
        if !response.status().is_success() {
            return Err(AppError::Api(format!("search failed: {}", response.status())));
        }
        let raw: Vec<ModSearchResult> = response.json().await?;
        Ok(raw.into_iter().map(Into::into).collect())
    }

    /// Get full mod metadata. Phase 3.1.
    pub async fn get_mod_details(&self, mod_id: i32) -> AppResult<Mod> {
        let path = format!("/games/{}/mods/{mod_id}.json", self.game_id);
        let response = self.request(reqwest::Method::GET, &path, &[]).await?;
        self.update_rate_limit(&response);
        if !response.status().is_success() {
            return Err(AppError::Api(format!(
                "get_mod_details failed: {}",
                response.status()
            )));
        }
        let raw: ModDetails = response.json().await?;
        Ok(raw.into())
    }

    /// List files attached to a mod. Phase 3.1.
    pub async fn get_mod_files(&self, mod_id: i32) -> AppResult<Vec<ModFile>> {
        let path = format!("/games/{}/mods/{mod_id}/files.json", self.game_id);
        let response = self.request(reqwest::Method::GET, &path, &[]).await?;
        self.update_rate_limit(&response);
        if !response.status().is_success() {
            return Err(AppError::Api(format!(
                "get_mod_files failed: {}",
                response.status()
            )));
        }
        Ok(response.json().await?)
    }

    /// Generate a download URL for a mod file. Phase 3.5.
    pub async fn generate_download_url(
        &self,
        mod_id: i32,
        file_id: i32,
        nxm: Option<&NxmLink>,
    ) -> AppResult<String> {
        // The Nexus API expects either a key/expires from the nxm:// link, or
        // a user-bound download. We use the nxm key when present, otherwise
        // request a one-click key via the standard endpoint.
        let mut path = format!(
            "/games/{}/mods/{mod_id}/files/{file_id}/download.json",
            self.game_id
        );
        if let Some(n) = nxm {
            if let (Some(k), Some(e)) = (&n.key, n.expires) {
                path.push_str(&format!("?key={k}&expires={e}"));
            }
        }
        let response = self.request(reqwest::Method::GET, &path, &[]).await?;
        self.update_rate_limit(&response);
        if !response.status().is_success() {
            return Err(AppError::Api(format!(
                "generate_download_url failed: {}",
                response.status()
            )));
        }
        let parsed: Vec<DownloadLink> = response.json().await?;
        parsed
            .into_iter()
            .next()
            .map(|d| d.uri)
            .ok_or_else(|| AppError::Api("download response empty".into()))
    }

    /// Parse an `nxm://game_domain/mods/mod_id/files/file_id?key=...&expires=...` link.
    /// Phase 3.5.
    pub fn parse_nxm(uri: &str) -> AppResult<NxmLink> {
        let url = url::Url::parse(uri)?;
        if url.scheme() != "nxm" {
            return Err(AppError::Api(format!("not an nxm:// URI: {uri}")));
        }
        let segments: Vec<&str> = url
            .path()
            .trim_start_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        if segments.len() != 4 || segments[0] != "mods" || segments[2] != "files" {
            return Err(AppError::Api(format!("malformed nxm:// path: {uri}")));
        }
        let game_domain = url.host_str().unwrap_or("").to_string();
        let mod_id: i32 = segments[1]
            .parse()
            .map_err(|_| AppError::Api(format!("bad mod_id in nxm:// link: {}", segments[1])))?;
        let file_id: i32 = segments[3]
            .parse()
            .map_err(|_| AppError::Api(format!("bad file_id in nxm:// link: {}", segments[3])))?;

        let mut key = None;
        let mut expires = None;
        for (k, v) in url.query_pairs() {
            match &*k {
                "key" => key = Some(v.into_owned()),
                "expires" => expires = v.parse().ok(),
                _ => {}
            }
        }

        Ok(NxmLink {
            game_domain,
            mod_id,
            file_id,
            key,
            expires,
        })
    }

    /// Build a `reqwest::RequestBuilder` with auth + UA headers, optionally retrying once
    /// on 429 (Phase 3.4). Always honors `Retry-After` if present.
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        attempts: &[u64],
    ) -> AppResult<reqwest::Response> {
        let url = format!("{}{path}", self.base_url);
        let mut attempt_index = 0usize;
        loop {
            let result = self
                .client
                .request(method.clone(), &url)
                .header("X-User-Agent", "nexus-cp2077-mod-manager")
                .header("apikey", &self.api_key)
                .header("Accept", "application/json")
                .send()
                .await;
            match result {
                Ok(resp) => {
                    if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                        let retry_after = resp
                            .headers()
                            .get("retry-after")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                            .unwrap_or(2);
                        tracing::warn!(retry_after, "Nexus 429 — backing off");
                        tokio::time::sleep(Duration::from_secs(retry_after + 1)).await;
                        if attempts.get(attempt_index).is_some() {
                            attempt_index += 1;
                            continue;
                        }
                        return Ok(resp);
                    }
                    return Ok(resp);
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Nexus request error");
                    if attempts.get(attempt_index).is_some() {
                        attempt_index += 1;
                        continue;
                    }
                    return Err(e.into());
                }
            }
        }
    }

    fn update_rate_limit(&self, response: &reqwest::Response) {
        if let Some(v) = response
            .headers()
            .get("x-rl-hourly-remaining")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            self.rate_limit.lock().expect("rate_limit mutex").hourly_remaining = Some(v);
        }
        if let Some(v) = response
            .headers()
            .get("x-rl-daily-remaining")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            self.rate_limit.lock().expect("rate_limit mutex").daily_remaining = Some(v);
        }
        if let Some(v) = response
            .headers()
            .get("x-rl-hourly-limit")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            self.rate_limit.lock().expect("rate_limit mutex").hourly_limit = Some(v);
        }
        if let Some(v) = response
            .headers()
            .get("x-rl-daily-limit")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok())
        {
            self.rate_limit.lock().expect("rate_limit mutex").daily_limit = Some(v);
        }
    }
}

// --- Wire-format DTOs (private) --------------------------------------------

#[derive(Debug, Deserialize)]
struct ModSearchResult {
    #[serde(rename = "mod_id")]
    mod_id: i32,
    name: String,
    version: Option<String>,
    summary: Option<String>,
    #[serde(rename = "category_id")]
    category_id: Option<i32>,
}

impl From<ModSearchResult> for Mod {
    fn from(r: ModSearchResult) -> Self {
        Mod {
            id: 0,
            name: r.name,
            version: r.version.unwrap_or_default(),
            description: r.summary.unwrap_or_default(),
            category: r
                .category_id
                .map(|c| c.to_string())
                .unwrap_or_default(),
            nexus_id: r.mod_id.to_string(),
            required_mod_ids: Vec::new(),
            required_versions: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ModDetails {
    id: i32,
    name: String,
    version: Option<String>,
    description: Option<String>,
    #[serde(rename = "category_id")]
    category_id: Option<i32>,
    #[serde(default)]
    mod_requirements: Vec<String>,
}

impl From<ModDetails> for Mod {
    fn from(d: ModDetails) -> Self {
        Mod {
            id: 0,
            name: d.name,
            version: d.version.unwrap_or_default(),
            description: d.description.unwrap_or_default(),
            category: d.category_id.map(|c| c.to_string()).unwrap_or_default(),
            nexus_id: d.id.to_string(),
            required_mod_ids: d.mod_requirements,
            required_versions: std::collections::HashMap::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct DownloadLink {
    #[serde(rename = "URI")]
    uri: String,
}

// --- URL helpers -----------------------------------------------------------

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{:02X}", b));
            }
        }
    }
    out
}
