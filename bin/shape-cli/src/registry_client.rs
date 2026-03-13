use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::{self, DEFAULT_REGISTRY};

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
    #[serde(default)]
    pub has_native_deps: bool,
    #[serde(default)]
    pub native_platforms: Vec<String>,
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
    #[serde(default)]
    pub has_native_deps: bool,
    #[serde(default)]
    pub native_platforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterResponse {
    pub username: String,
    pub token: String,
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
        let config_dir =
            config::shape_config_dir().ok_or("could not determine config directory")?;
        Ok(config_dir.join("credentials.json"))
    }

    /// Load credentials from ~/.shape/credentials.json
    pub fn load_credentials() -> Result<Credentials, String> {
        let path = Self::credentials_path()?;
        let contents = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read credentials from {}: {}", path.display(), e))?;
        serde_json::from_str(&contents)
            .map_err(|e| format!("failed to parse credentials from {}: {}", path.display(), e))
    }

    /// Save credentials to ~/.shape/credentials.json (mode 0600)
    pub fn save_credentials(credentials: &Credentials) -> Result<(), String> {
        let path = Self::credentials_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create directory {}: {}", parent.display(), e))?;
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

    /// Register a new account: POST /v1/api/auth/register
    pub async fn register(
        &self,
        username: &str,
        email: &str,
        password: &str,
    ) -> Result<RegisterResponse, String> {
        let url = format!("{}/v1/api/auth/register", self.registry_url);
        let body = serde_json::json!({
            "username": username,
            "email": email,
            "password": password,
        });
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("register request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "registration failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        resp.json::<RegisterResponse>()
            .await
            .map_err(|e| format!("failed to parse register response: {}", e))
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

    /// Validate token: GET /v1/api/auth/validate (requires auth)
    ///
    /// Makes a lightweight request to verify the token is valid.
    /// Returns Ok(()) if the token is accepted, Err otherwise.
    pub async fn validate_token(&self) -> Result<(), String> {
        let token = self.auth_header()?;
        let url = format!("{}/v1/api/auth/validate", self.registry_url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|e| format!("token validation request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "token validation failed with status {}: {}",
                resp.status(),
                resp.text()
                    .await
                    .unwrap_or_else(|_| "unknown error".to_string())
            ));
        }

        Ok(())
    }

    /// Publish via multipart: POST /v1/api/packages/new (requires auth)
    pub async fn publish_multipart(
        &self,
        shapec_bytes: Vec<u8>,
        source_bytes: Option<Vec<u8>>,
        native_blobs: Vec<(String, Vec<u8>)>,
    ) -> Result<String, String> {
        let token = self.auth_header()?;
        let url = format!("{}/v1/api/packages/new", self.registry_url);

        let mut form = reqwest::multipart::Form::new().part(
            "shapec",
            reqwest::multipart::Part::bytes(shapec_bytes)
                .mime_str("application/octet-stream")
                .unwrap(),
        );

        if let Some(source) = source_bytes {
            form = form.part(
                "source",
                reqwest::multipart::Part::bytes(source)
                    .mime_str("application/gzip")
                    .unwrap(),
            );
        }

        for (target, data) in native_blobs {
            form = form.part(
                format!("native:{target}"),
                reqwest::multipart::Part::bytes(data)
                    .mime_str("application/gzip")
                    .unwrap(),
            );
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
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

    /// Download source tarball: GET /v1/api/packages/{name}/{version}/download/source
    pub async fn download_source(&self, name: &str, version: &str) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/v1/api/packages/{}/{}/download/source",
            self.registry_url, name, version
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download source request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "download source failed with status {}: {}",
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

    /// Download native blob: GET /v1/api/packages/{name}/{version}/download/native/{target}
    pub async fn download_native(
        &self,
        name: &str,
        version: &str,
        target: &str,
    ) -> Result<Vec<u8>, String> {
        let url = format!(
            "{}/v1/api/packages/{}/{}/download/native/{}",
            self.registry_url, name, version, target
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("download native request failed: {}", e))?;

        if !resp.status().is_success() {
            return Err(format!(
                "download native failed with status {}: {}",
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

    /// Legacy publish bundle: POST /v1/api/packages/new (requires auth)
    pub async fn publish(&self, bundle_bytes: Vec<u8>) -> Result<String, String> {
        let token = self.auth_header()?;
        let url = format!("{}/v1/api/packages/new", self.registry_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token))
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
            .header("Authorization", format!("Bearer {}", token))
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
            .header("Authorization", format!("Bearer {}", token))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_header_no_token() {
        let client = RegistryClient::new(None);
        let result = client.auth_header();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not authenticated"));
    }

    #[test]
    fn test_auth_header_with_token() {
        let client = RegistryClient::new(None).with_token("test-token-12345678".to_string());
        let result = client.auth_header();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "test-token-12345678");
    }

    #[test]
    fn test_default_registry_url() {
        let client = RegistryClient::new(None);
        assert_eq!(client.registry_url, "https://pkg.shape-lang.dev");
    }

    #[test]
    fn test_custom_registry_url() {
        let client = RegistryClient::new(Some("https://custom.registry.io".to_string()));
        assert_eq!(client.registry_url, "https://custom.registry.io");
    }

    #[test]
    fn test_publish_requires_auth() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = RegistryClient::new(None); // no token
        let result = rt.block_on(client.publish(vec![1, 2, 3]));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not authenticated"));
    }

    #[test]
    fn test_validate_token_requires_auth() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let client = RegistryClient::new(None); // no token
        let result = rt.block_on(client.validate_token());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not authenticated"));
    }

    #[test]
    fn test_credentials_serialization() {
        let creds = Credentials {
            registry: "https://test.example.com".to_string(),
            token: "test-token-abcdefgh".to_string(),
        };
        let json = serde_json::to_string(&creds).unwrap();
        let deserialized: Credentials = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.registry, creds.registry);
        assert_eq!(deserialized.token, creds.token);
    }
}
