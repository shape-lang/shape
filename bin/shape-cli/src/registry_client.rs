use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const DEFAULT_REGISTRY: &str = "https://pkg.shape-lang.dev";

#[derive(Debug, Serialize, Deserialize)]
pub struct Credentials {
    pub registry: String,
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct PackageSearchResult {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub downloads: u64,
}

#[derive(Debug, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub description: Option<String>,
    pub versions: Vec<VersionInfo>,
    pub owners: Vec<String>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub downloads: u64,
}

#[derive(Debug, Deserialize)]
pub struct VersionInfo {
    pub version: String,
    pub author_key: Option<String>,
    pub required_permissions: Vec<String>,
    pub bundle_sha256: Option<String>,
    pub bundle_size: Option<u64>,
    pub yanked: bool,
    pub published_at: String,
    pub downloads: u64,
}

pub struct RegistryClient {
    client: reqwest::Client,
    registry_url: String,
    token: Option<String>,
}

impl RegistryClient {
    pub fn new(registry_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            registry_url: registry_url.unwrap_or_else(|| DEFAULT_REGISTRY.to_string()),
            token: None,
        }
    }

    pub fn with_token(mut self, token: String) -> Self {
        self.token = Some(token);
        self
    }

    fn credentials_path() -> Result<PathBuf, String> {
        let home = dirs::home_dir().ok_or("could not determine home directory")?;
        Ok(home.join(".shape").join("credentials.json"))
    }

    /// Load credentials from ~/.shape/credentials.json
    pub fn load_credentials() -> Result<Credentials, String> {
        let path = Self::credentials_path()?;
        let contents = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "failed to read credentials from {}: {}",
                path.display(),
                e
            )
        })?;
        serde_json::from_str(&contents).map_err(|e| {
            format!(
                "failed to parse credentials from {}: {}",
                path.display(),
                e
            )
        })
    }

    /// Save credentials to ~/.shape/credentials.json (mode 0600)
    pub fn save_credentials(credentials: &Credentials) -> Result<(), String> {
        let path = Self::credentials_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!("failed to create directory {}: {}", parent.display(), e)
            })?;
        }
        let json = serde_json::to_string_pretty(credentials)
            .map_err(|e| format!("failed to serialize credentials: {}", e))?;
        std::fs::write(&path, &json)
            .map_err(|e| format!("failed to write credentials to {}: {}", path.display(), e))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("failed to set permissions on {}: {}", path.display(), e))?;
        }

        Ok(())
    }

    fn auth_header(&self) -> Result<String, String> {
        self.token
            .clone()
            .ok_or_else(|| "not authenticated: no token set (run `shape login` first)".to_string())
    }

    /// Search packages: GET /v1/api/packages?q=<query>
    pub async fn search(&self, query: &str) -> Result<Vec<PackageSearchResult>, String> {
        let url = format!("{}/v1/api/packages", self.registry_url);
        let resp = self
            .client
            .get(&url)
            .query(&[("q", query)])
            .send()
            .await
            .map_err(|e| format!("search request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "search failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        resp.json::<Vec<PackageSearchResult>>()
            .await
            .map_err(|e| format!("failed to parse search results: {}", e))
    }

    /// Get package info: GET /v1/api/packages/{name}
    pub async fn get_info(&self, name: &str) -> Result<PackageInfo, String> {
        let url = format!("{}/v1/api/packages/{}", self.registry_url, name);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("get info request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "get info failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        resp.json::<PackageInfo>()
            .await
            .map_err(|e| format!("failed to parse package info: {}", e))
    }

    /// Download bundle: GET /v1/api/packages/{name}/{version}/download
    pub async fn download_bundle(&self, name: &str, version: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/v1/api/packages/{}/{}/download",
            self.registry_url, name, version
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "download failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("failed to read download body: {}", e))
    }

    /// Fetch sparse index: GET /v1/index/{name}
    pub async fn fetch_index(&self, name: &str) -> Result<String, String> {
        let url = format!("{}/v1/index/{}", self.registry_url, name);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("fetch index request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "fetch index failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        resp.text()
            .await
            .map_err(|e| format!("failed to read index body: {}", e))
    }

    /// Publish bundle: POST /v1/api/packages/new (requires auth)
    pub async fn publish(&self, bundle_bytes: Vec<u8>) -> Result<String, String> {
        let token = self.auth_header()?;
        let url = format!("{}/v1/api/packages/new", self.registry_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", &token)
            .header("Content-Type", "application/octet-stream")
            .body(bundle_bytes)
            .send()
            .await
            .map_err(|e| format!("publish request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "publish failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        resp.text()
            .await
            .map_err(|e| format!("failed to read publish response: {}", e))
    }

    /// Yank version: DELETE /v1/api/packages/{name}/{version}/yank (requires auth)
    pub async fn yank(&self, name: &str, version: &str) -> Result<(), String> {
        let token = self.auth_header()?;
        let url = format!(
            "{}/v1/api/packages/{}/{}/yank",
            self.registry_url, name, version
        );
        let resp = self
            .client
            .delete(&url)
            .header("Authorization", &token)
            .send()
            .await
            .map_err(|e| format!("yank request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "yank failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        Ok(())
    }

    /// Unyank version: PUT /v1/api/packages/{name}/{version}/unyank (requires auth)
    pub async fn unyank(&self, name: &str, version: &str) -> Result<(), String> {
        let token = self.auth_header()?;
        let url = format!(
            "{}/v1/api/packages/{}/{}/unyank",
            self.registry_url, name, version
        );
        let resp = self
            .client
            .put(&url)
            .header("Authorization", &token)
            .send()
            .await
            .map_err(|e| format!("unyank request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "unyank failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        Ok(())
    }
}
