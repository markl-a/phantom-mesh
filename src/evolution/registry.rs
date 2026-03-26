use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::{debug, info};

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("package not found: {0}")]
    NotFound(String),
    #[error("verification failed: {0}")]
    VerificationFailed(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
}

pub type RegistryResult<T> = Result<T, RegistryError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PackageType {
    Skill,
    Plugin,
    Hand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersion {
    pub version: String,
    pub checksum_sha256: String,
    pub download_url: String,
    pub size_bytes: u64,
    pub released_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub package_type: PackageType,
    pub capabilities: Vec<String>,
    pub author: String,
    pub verified: bool,
    pub versions: Vec<PackageVersion>,
}

impl PackageInfo {
    pub fn latest_version(&self) -> Option<&PackageVersion> {
        self.versions.last()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub packages: Vec<PackageInfo>,
    pub updated_at: String,
}

impl RegistryIndex {
    pub fn search_by_capability(&self, capability: &str) -> Vec<&PackageInfo> {
        self.packages
            .iter()
            .filter(|p| p.capabilities.iter().any(|c| c == capability))
            .collect()
    }

    pub fn search_by_name(&self, query: &str) -> Vec<&PackageInfo> {
        let q = query.to_lowercase();
        self.packages
            .iter()
            .filter(|p| p.name.to_lowercase().contains(&q) || p.id.to_lowercase().contains(&q))
            .collect()
    }

    pub fn get_package(&self, id: &str) -> Option<&PackageInfo> {
        self.packages.iter().find(|p| p.id == id)
    }
}

/// Trait for pluggable package registries.
#[async_trait]
pub trait PackageRegistry: Send + Sync {
    async fn fetch_index(&self) -> RegistryResult<RegistryIndex>;
    async fn download(&self, id: &str, version: &str) -> RegistryResult<Vec<u8>>;
    async fn verify(&self, data: &[u8], expected_sha256: &str) -> RegistryResult<bool>;
}

/// Verify SHA-256 checksum of data.
pub fn verify_sha256(data: &[u8], expected: &str) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hex::encode(hasher.finalize());
    result == expected
}

/// HTTP-based package registry (fetches JSON index from URL).
pub struct HttpRegistry {
    base_url: String,
    client: reqwest::Client,
}

impl HttpRegistry {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        }
    }
}

#[async_trait]
impl PackageRegistry for HttpRegistry {
    async fn fetch_index(&self) -> RegistryResult<RegistryIndex> {
        let url = format!("{}/index.json", self.base_url);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| RegistryError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(RegistryError::Network(format!("HTTP {}", resp.status())));
        }
        let index: RegistryIndex = resp
            .json()
            .await
            .map_err(|e| RegistryError::Parse(e.to_string()))?;
        info!(
            "HttpRegistry: fetched index with {} packages",
            index.packages.len()
        );
        Ok(index)
    }

    async fn download(&self, id: &str, version: &str) -> RegistryResult<Vec<u8>> {
        let url = format!("{}/packages/{}/{}.tar.gz", self.base_url, id, version);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| RegistryError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(RegistryError::NotFound(format!("{}@{}", id, version)));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RegistryError::Network(e.to_string()))?;
        debug!(
            "HttpRegistry: downloaded {}@{} ({} bytes)",
            id,
            version,
            bytes.len()
        );
        Ok(bytes.to_vec())
    }

    async fn verify(&self, data: &[u8], expected_sha256: &str) -> RegistryResult<bool> {
        Ok(verify_sha256(data, expected_sha256))
    }
}

/// Local filesystem-based registry (for offline mode).
pub struct LocalRegistry {
    base_path: PathBuf,
}

impl LocalRegistry {
    pub fn new(base_path: &Path) -> Self {
        Self {
            base_path: base_path.to_path_buf(),
        }
    }
}

#[async_trait]
impl PackageRegistry for LocalRegistry {
    async fn fetch_index(&self) -> RegistryResult<RegistryIndex> {
        let index_path = self.base_path.join("index.json");
        let data = tokio::fs::read_to_string(&index_path).await?;
        let index: RegistryIndex =
            serde_json::from_str(&data).map_err(|e| RegistryError::Parse(e.to_string()))?;
        info!(
            "LocalRegistry: loaded index with {} packages from {}",
            index.packages.len(),
            index_path.display()
        );
        Ok(index)
    }

    async fn download(&self, id: &str, version: &str) -> RegistryResult<Vec<u8>> {
        // Validate no path traversal
        if id.contains("..") || id.contains('/') || id.contains('\\')
            || version.contains("..") || version.contains('/') || version.contains('\\')
        {
            return Err(RegistryError::NotFound(format!(
                "invalid id or version: {}@{}", id, version
            )));
        }
        let pkg_path = self
            .base_path
            .join("packages")
            .join(id)
            .join(format!("{}.tar.gz", version));
        if !pkg_path.exists() {
            return Err(RegistryError::NotFound(format!(
                "{}@{} at {}",
                id,
                version,
                pkg_path.display()
            )));
        }
        let data = tokio::fs::read(&pkg_path).await?;
        debug!(
            "LocalRegistry: loaded {}@{} ({} bytes)",
            id,
            version,
            data.len()
        );
        Ok(data)
    }

    async fn verify(&self, data: &[u8], expected_sha256: &str) -> RegistryResult<bool> {
        Ok(verify_sha256(data, expected_sha256))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_version(ver: &str) -> PackageVersion {
        PackageVersion {
            version: ver.to_string(),
            checksum_sha256: "abc123".to_string(),
            download_url: format!("https://example.com/{}.tar.gz", ver),
            size_bytes: 1024,
            released_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn sample_package(id: &str, name: &str, caps: Vec<&str>) -> PackageInfo {
        PackageInfo {
            id: id.to_string(),
            name: name.to_string(),
            description: format!("A test package: {}", name),
            package_type: PackageType::Skill,
            capabilities: caps.into_iter().map(|s| s.to_string()).collect(),
            author: "tester".to_string(),
            verified: true,
            versions: vec![sample_version("0.1.0"), sample_version("0.2.0")],
        }
    }

    fn sample_index() -> RegistryIndex {
        RegistryIndex {
            packages: vec![
                sample_package("web-search", "Web Search", vec!["search", "web"]),
                sample_package("code-gen", "Code Generator", vec!["code", "generate"]),
                sample_package("translate", "Translator", vec!["translate", "language"]),
            ],
            updated_at: "2026-03-22T00:00:00Z".to_string(),
        }
    }

    // --- verify_sha256 ---

    #[test]
    fn test_verify_sha256_correct() {
        let data = b"hello world";
        // known SHA-256 of "hello world"
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(verify_sha256(data, expected));
    }

    #[test]
    fn test_verify_sha256_incorrect() {
        let data = b"hello world";
        let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
        assert!(!verify_sha256(data, wrong));
    }

    #[test]
    fn test_verify_sha256_empty() {
        let data = b"";
        // SHA-256 of empty input
        let expected = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(verify_sha256(data, expected));
    }

    // --- PackageInfo::latest_version ---

    #[test]
    fn test_latest_version() {
        let pkg = sample_package("test", "Test", vec![]);
        let latest = pkg.latest_version().unwrap();
        assert_eq!(latest.version, "0.2.0");
    }

    #[test]
    fn test_latest_version_empty() {
        let mut pkg = sample_package("test", "Test", vec![]);
        pkg.versions.clear();
        assert!(pkg.latest_version().is_none());
    }

    // --- RegistryIndex searches ---

    #[test]
    fn test_search_by_capability_found() {
        let index = sample_index();
        let results = index.search_by_capability("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "web-search");
    }

    #[test]
    fn test_search_by_capability_not_found() {
        let index = sample_index();
        let results = index.search_by_capability("nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_by_name() {
        let index = sample_index();
        let results = index.search_by_name("code");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "code-gen");
    }

    #[test]
    fn test_search_by_name_case_insensitive() {
        let index = sample_index();
        let results = index.search_by_name("TRANSLATOR");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "translate");
    }

    #[test]
    fn test_search_by_name_matches_id() {
        let index = sample_index();
        let results = index.search_by_name("web-search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "web-search");
    }

    #[test]
    fn test_search_by_name_not_found() {
        let index = sample_index();
        let results = index.search_by_name("zzzzz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_package_found() {
        let index = sample_index();
        let pkg = index.get_package("translate").unwrap();
        assert_eq!(pkg.name, "Translator");
    }

    #[test]
    fn test_get_package_not_found() {
        let index = sample_index();
        assert!(index.get_package("nonexistent").is_none());
    }

    // --- LocalRegistry integration test ---

    #[tokio::test]
    async fn test_local_registry_fetch_index() {
        let dir = tempfile::tempdir().unwrap();
        let index = sample_index();
        let json = serde_json::to_string_pretty(&index).unwrap();
        tokio::fs::write(dir.path().join("index.json"), &json)
            .await
            .unwrap();

        let registry = LocalRegistry::new(dir.path());
        let loaded = registry.fetch_index().await.unwrap();
        assert_eq!(loaded.packages.len(), 3);
        assert_eq!(loaded.packages[0].id, "web-search");
    }

    #[tokio::test]
    async fn test_local_registry_download() {
        let dir = tempfile::tempdir().unwrap();
        let pkg_dir = dir.path().join("packages").join("test-pkg");
        tokio::fs::create_dir_all(&pkg_dir).await.unwrap();
        let pkg_data = b"fake-tarball-content";
        tokio::fs::write(pkg_dir.join("1.0.0.tar.gz"), pkg_data)
            .await
            .unwrap();

        let registry = LocalRegistry::new(dir.path());
        let data = registry.download("test-pkg", "1.0.0").await.unwrap();
        assert_eq!(data, pkg_data);
    }

    #[tokio::test]
    async fn test_local_registry_download_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let registry = LocalRegistry::new(dir.path());
        let result = registry.download("no-such-pkg", "1.0.0").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, RegistryError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_local_registry_verify() {
        let registry = LocalRegistry::new(Path::new("/tmp"));
        let data = b"hello world";
        let hash = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert!(registry.verify(data, hash).await.unwrap());
        assert!(!registry.verify(data, "bad-hash").await.unwrap());
    }

    #[tokio::test]
    async fn test_local_registry_fetch_index_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let registry = LocalRegistry::new(dir.path());
        let result = registry.fetch_index().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, RegistryError::Io(_)));
    }

    #[tokio::test]
    async fn test_local_registry_fetch_index_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        tokio::fs::write(dir.path().join("index.json"), "not-valid-json")
            .await
            .unwrap();
        let registry = LocalRegistry::new(dir.path());
        let result = registry.fetch_index().await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, RegistryError::Parse(_)));
    }

    #[tokio::test]
    async fn test_local_registry_download_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let registry = LocalRegistry::new(dir.path());

        // Attempt path traversal via id
        let result = registry.download("../../../etc", "passwd").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RegistryError::NotFound(_)));

        // Attempt path traversal via version
        let result = registry.download("legit-pkg", "../../secret").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RegistryError::NotFound(_)));

        // Attempt path traversal with backslashes
        let result = registry.download("pkg", "..\\..\\secret").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RegistryError::NotFound(_)));

        // Attempt with forward slash in id
        let result = registry.download("pkg/subdir", "1.0.0").await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), RegistryError::NotFound(_)));
    }
}
