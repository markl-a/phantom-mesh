//! Disaster Recovery System — backup, restore, and health-check for all SQLite databases.
//!
//! Provides:
//! - `BackupManager` — schedule-aware backup of all 6 SQLite DBs to timestamped dirs
//! - `restore_from()` — checksum-verified restore from a backup manifest
//! - `check_db_integrity()` — `PRAGMA integrity_check` on each DB
//!
//! DBs covered: core.db, costs.db, memory.db, knowledge.db, revenue.db, trajectories.db

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// The six SQLite databases managed by clawtex-core.
pub const MANAGED_DBS: &[&str] = &[
    "core.db",
    "costs.db",
    "memory.db",
    "knowledge.db",
    "revenue.db",
    "trajectories.db",
];

/// Default number of backups to retain.
const DEFAULT_MAX_BACKUPS: usize = 7;

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

/// Result of backing up a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupFileEntry {
    /// Original filename (e.g. "core.db").
    pub filename: String,
    /// Size in bytes of the backed-up copy.
    pub size_bytes: u64,
    /// Hex-encoded SHA-256 checksum.
    pub sha256: String,
}

/// A manifest describing a complete backup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    /// ISO-8601 timestamp of the backup.
    pub timestamp: DateTime<Utc>,
    /// Directory containing the backup files.
    pub backup_dir: String,
    /// Per-file entries.
    pub files: Vec<BackupFileEntry>,
    /// Total size of all backed-up files.
    pub total_size_bytes: u64,
}

/// Status of restoring a single file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestoreFileStatus {
    /// File was restored successfully.
    Ok,
    /// Checksum mismatch — file was NOT restored.
    ChecksumMismatch { expected: String, actual: String },
    /// Source file not found in backup directory.
    NotFound,
    /// An I/O or other error occurred.
    Error(String),
}

/// Report on a single file's restore outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreFileResult {
    pub filename: String,
    pub status: RestoreFileStatus,
}

/// Overall restore report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreReport {
    pub manifest_path: String,
    pub results: Vec<RestoreFileResult>,
    pub all_ok: bool,
}

/// Health status of a single database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DbHealthStatus {
    Ok,
    Corrupted(String),
    Missing,
    OpenError(String),
}

/// Health check result for a single database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbHealthResult {
    pub db_name: String,
    pub status: DbHealthStatus,
    pub size_bytes: u64,
}

/// Backup schedule configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSchedule {
    /// Cron expression (informational — actual scheduling is external).
    pub cron_expr: String,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self {
            cron_expr: "0 0 * * *".to_string(), // daily at midnight
        }
    }
}

// ---------------------------------------------------------------------------
// BackupManager
// ---------------------------------------------------------------------------

/// Manages backup, restore, and integrity checking of clawtex SQLite databases.
pub struct BackupManager {
    /// Directory containing the source databases (e.g. `~/.clawtex/`).
    pub db_dir: PathBuf,
    /// Root directory for backup storage.
    pub backup_dir: PathBuf,
    /// Backup schedule (informational).
    pub schedule: BackupSchedule,
    /// Maximum number of backup sets to retain.
    pub max_backups: usize,
}

impl BackupManager {
    /// Create a new BackupManager.
    pub fn new(db_dir: impl Into<PathBuf>, backup_dir: impl Into<PathBuf>) -> Self {
        Self {
            db_dir: db_dir.into(),
            backup_dir: backup_dir.into(),
            schedule: BackupSchedule::default(),
            max_backups: DEFAULT_MAX_BACKUPS,
        }
    }

    /// Set the maximum number of backups to retain.
    pub fn with_max_backups(mut self, n: usize) -> Self {
        self.max_backups = n;
        self
    }

    /// Set the backup schedule.
    pub fn with_schedule(mut self, schedule: BackupSchedule) -> Self {
        self.schedule = schedule;
        self
    }

    // -----------------------------------------------------------------------
    // Backup
    // -----------------------------------------------------------------------

    /// Back up all managed SQLite databases to a timestamped subdirectory.
    ///
    /// Returns a `BackupManifest` describing the backup.  Only databases that
    /// actually exist on disk are copied — missing ones are silently skipped
    /// (they may not have been created yet).
    pub fn backup_all(&self) -> Result<BackupManifest> {
        let now = Utc::now();
        let ts_dir_name = now.format("%Y%m%d_%H%M%S").to_string();
        let target_dir = self.backup_dir.join(&ts_dir_name);

        fs::create_dir_all(&target_dir)
            .with_context(|| format!("Failed to create backup dir: {:?}", target_dir))?;

        let mut entries = Vec::new();
        let mut total_size: u64 = 0;

        for &db_name in MANAGED_DBS {
            let src = self.db_dir.join(db_name);
            if !src.exists() {
                debug!("Skipping missing DB: {:?}", src);
                continue;
            }

            let dst = target_dir.join(db_name);
            fs::copy(&src, &dst)
                .with_context(|| format!("Failed to copy {:?} -> {:?}", src, dst))?;

            let meta = fs::metadata(&dst)?;
            let size = meta.len();
            let checksum = sha256_file(&dst)?;

            entries.push(BackupFileEntry {
                filename: db_name.to_string(),
                size_bytes: size,
                sha256: checksum,
            });

            total_size += size;
            info!("Backed up {} ({} bytes)", db_name, size);
        }

        let manifest = BackupManifest {
            timestamp: now,
            backup_dir: target_dir.to_string_lossy().to_string(),
            files: entries,
            total_size_bytes: total_size,
        };

        // Write manifest JSON
        let manifest_path = target_dir.join("manifest.json");
        let json = serde_json::to_string_pretty(&manifest)?;
        fs::write(&manifest_path, &json)
            .with_context(|| format!("Failed to write manifest: {:?}", manifest_path))?;

        info!(
            "Backup complete: {} files, {} bytes total -> {:?}",
            manifest.files.len(),
            total_size,
            target_dir
        );

        // Auto-cleanup old backups
        if let Err(e) = self.cleanup_old_backups() {
            warn!("Failed to cleanup old backups: {}", e);
        }

        Ok(manifest)
    }

    /// Remove old backup directories, keeping only the most recent `max_backups`.
    fn cleanup_old_backups(&self) -> Result<()> {
        if !self.backup_dir.exists() {
            return Ok(());
        }

        let mut backup_dirs: Vec<(PathBuf, SystemTime)> = Vec::new();

        for entry in fs::read_dir(&self.backup_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                // Check if it has a manifest.json — indicates a valid backup
                if path.join("manifest.json").exists() {
                    let modified = entry.metadata()?.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    backup_dirs.push((path, modified));
                }
            }
        }

        // Sort newest first
        backup_dirs.sort_by(|a, b| b.1.cmp(&a.1));

        // Remove excess
        if backup_dirs.len() > self.max_backups {
            for (dir, _) in &backup_dirs[self.max_backups..] {
                info!("Removing old backup: {:?}", dir);
                fs::remove_dir_all(dir)
                    .with_context(|| format!("Failed to remove old backup: {:?}", dir))?;
            }
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Restore
    // -----------------------------------------------------------------------

    /// Restore databases from a backup manifest file.
    ///
    /// Verifies SHA-256 checksums before copying each file.  Files with
    /// mismatched checksums are NOT restored.
    pub fn restore_from(&self, manifest_path: &str) -> Result<RestoreReport> {
        let manifest_content = fs::read_to_string(manifest_path)
            .with_context(|| format!("Failed to read manifest: {}", manifest_path))?;

        let manifest: BackupManifest = serde_json::from_str(&manifest_content)
            .with_context(|| "Failed to parse backup manifest JSON")?;

        let backup_src_dir = Path::new(&manifest.backup_dir);
        let mut results = Vec::new();
        let mut all_ok = true;

        for entry in &manifest.files {
            let src = backup_src_dir.join(&entry.filename);
            let dst = self.db_dir.join(&entry.filename);

            let status = if !src.exists() {
                all_ok = false;
                RestoreFileStatus::NotFound
            } else {
                match sha256_file(&src) {
                    Ok(actual_hash) => {
                        if actual_hash != entry.sha256 {
                            all_ok = false;
                            RestoreFileStatus::ChecksumMismatch {
                                expected: entry.sha256.clone(),
                                actual: actual_hash,
                            }
                        } else {
                            match fs::copy(&src, &dst) {
                                Ok(_) => {
                                    info!("Restored {}", entry.filename);
                                    RestoreFileStatus::Ok
                                }
                                Err(e) => {
                                    all_ok = false;
                                    RestoreFileStatus::Error(format!("Copy failed: {}", e))
                                }
                            }
                        }
                    }
                    Err(e) => {
                        all_ok = false;
                        RestoreFileStatus::Error(format!("Checksum failed: {}", e))
                    }
                }
            };

            results.push(RestoreFileResult {
                filename: entry.filename.clone(),
                status,
            });
        }

        Ok(RestoreReport {
            manifest_path: manifest_path.to_string(),
            results,
            all_ok,
        })
    }

    // -----------------------------------------------------------------------
    // Health Check
    // -----------------------------------------------------------------------

    /// Run `PRAGMA integrity_check` on each managed database.
    pub fn check_db_integrity(&self) -> Vec<DbHealthResult> {
        MANAGED_DBS
            .iter()
            .map(|&db_name| {
                let path = self.db_dir.join(db_name);
                if !path.exists() {
                    return DbHealthResult {
                        db_name: db_name.to_string(),
                        status: DbHealthStatus::Missing,
                        size_bytes: 0,
                    };
                }

                let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

                match Connection::open(&path) {
                    Ok(conn) => {
                        let status = match conn.query_row(
                            "PRAGMA integrity_check",
                            [],
                            |row| row.get::<_, String>(0),
                        ) {
                            Ok(result) if result == "ok" => DbHealthStatus::Ok,
                            Ok(result) => DbHealthStatus::Corrupted(result),
                            Err(e) => DbHealthStatus::Corrupted(format!("Query error: {}", e)),
                        };

                        DbHealthResult {
                            db_name: db_name.to_string(),
                            status,
                            size_bytes,
                        }
                    }
                    Err(e) => DbHealthResult {
                        db_name: db_name.to_string(),
                        status: DbHealthStatus::OpenError(e.to_string()),
                        size_bytes,
                    },
                }
            })
            .collect()
    }

    /// List existing backup manifests, sorted newest first.
    pub fn list_backups(&self) -> Result<Vec<BackupManifest>> {
        if !self.backup_dir.exists() {
            return Ok(Vec::new());
        }

        let mut manifests = Vec::new();

        for entry in fs::read_dir(&self.backup_dir)? {
            let entry = entry?;
            let path = entry.path();
            let manifest_file = path.join("manifest.json");
            if manifest_file.exists() {
                if let Ok(content) = fs::read_to_string(&manifest_file) {
                    if let Ok(m) = serde_json::from_str::<BackupManifest>(&content) {
                        manifests.push(m);
                    }
                }
            }
        }

        // Sort newest first
        manifests.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        Ok(manifests)
    }

    /// Get a summary of total backup storage used.
    pub fn storage_summary(&self) -> Result<BackupStorageSummary> {
        let manifests = self.list_backups()?;
        let total_bytes: u64 = manifests.iter().map(|m| m.total_size_bytes).sum();
        Ok(BackupStorageSummary {
            backup_count: manifests.len(),
            total_bytes,
            max_backups: self.max_backups,
        })
    }
}

/// Summary of backup storage usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupStorageSummary {
    pub backup_count: usize,
    pub total_bytes: u64,
    pub max_backups: usize,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Compute hex-encoded SHA-256 checksum of a file.
fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("Cannot open file for checksum: {:?}", path))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Helper: create a fake SQLite DB with a table and some data.
    fn create_fake_db(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS test_data (id INTEGER PRIMARY KEY, value TEXT);
             INSERT INTO test_data (value) VALUES ('hello');
             INSERT INTO test_data (value) VALUES ('world');",
        )
        .unwrap();
        path
    }

    /// Helper: create all 6 managed DBs in a temp dir.
    fn create_all_dbs(dir: &Path) {
        for &name in MANAGED_DBS {
            create_fake_db(dir, name);
        }
    }

    fn make_manager(db_dir: &Path, backup_dir: &Path) -> BackupManager {
        BackupManager::new(db_dir, backup_dir)
    }

    // -- Backup Tests -------------------------------------------------------

    #[test]
    fn test_backup_all_creates_manifest() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_all_dbs(&db_dir);

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        assert_eq!(manifest.files.len(), 6);
        assert!(manifest.total_size_bytes > 0);

        // Manifest file should exist on disk
        let manifest_path = Path::new(&manifest.backup_dir).join("manifest.json");
        assert!(manifest_path.exists());
    }

    #[test]
    fn test_backup_skips_missing_dbs() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        // Only create 2 of the 6 DBs
        create_fake_db(&db_dir, "core.db");
        create_fake_db(&db_dir, "memory.db");

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        assert_eq!(manifest.files.len(), 2);
        let names: Vec<&str> = manifest.files.iter().map(|f| f.filename.as_str()).collect();
        assert!(names.contains(&"core.db"));
        assert!(names.contains(&"memory.db"));
    }

    #[test]
    fn test_backup_checksums_are_valid() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_fake_db(&db_dir, "core.db");

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        let entry = &manifest.files[0];
        assert_eq!(entry.filename, "core.db");
        assert!(!entry.sha256.is_empty());
        assert_eq!(entry.sha256.len(), 64); // SHA-256 hex = 64 chars

        // Verify the checksum matches the backed-up file
        let backed_up = Path::new(&manifest.backup_dir).join("core.db");
        let actual = sha256_file(&backed_up).unwrap();
        assert_eq!(actual, entry.sha256);
    }

    #[test]
    fn test_backup_cleanup_keeps_max() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_fake_db(&db_dir, "core.db");

        let mgr = make_manager(&db_dir, &backup_dir).with_max_backups(3);

        // Create 5 backups
        for _ in 0..5 {
            mgr.backup_all().unwrap();
            // Small delay to ensure different timestamp dir names
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        // Should only have 3 left
        let remaining = mgr.list_backups().unwrap();
        assert!(
            remaining.len() <= 3,
            "Expected at most 3 backups, got {}",
            remaining.len()
        );
    }

    #[test]
    fn test_backup_empty_db_dir() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        assert_eq!(manifest.files.len(), 0);
        assert_eq!(manifest.total_size_bytes, 0);
    }

    // -- Restore Tests ------------------------------------------------------

    #[test]
    fn test_restore_from_manifest() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        let restore_dir = tmp.path().join("restored");
        fs::create_dir_all(&db_dir).unwrap();
        fs::create_dir_all(&restore_dir).unwrap();

        create_all_dbs(&db_dir);

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        let manifest_path = Path::new(&manifest.backup_dir).join("manifest.json");

        // Restore into a different directory
        let restore_mgr = make_manager(&restore_dir, &backup_dir);
        let report = restore_mgr
            .restore_from(manifest_path.to_str().unwrap())
            .unwrap();

        assert!(report.all_ok);
        assert_eq!(report.results.len(), 6);
        for r in &report.results {
            assert_eq!(r.status, RestoreFileStatus::Ok);
        }

        // Verify all files exist in restore dir
        for &name in MANAGED_DBS {
            assert!(restore_dir.join(name).exists());
        }
    }

    #[test]
    fn test_restore_detects_checksum_mismatch() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_fake_db(&db_dir, "core.db");

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        // Corrupt the backed-up file
        let backed_up_path = Path::new(&manifest.backup_dir).join("core.db");
        fs::write(&backed_up_path, b"corrupted data").unwrap();

        let manifest_path = Path::new(&manifest.backup_dir).join("manifest.json");
        let report = mgr
            .restore_from(manifest_path.to_str().unwrap())
            .unwrap();

        assert!(!report.all_ok);
        assert_eq!(report.results.len(), 1);
        match &report.results[0].status {
            RestoreFileStatus::ChecksumMismatch { expected, actual } => {
                assert_ne!(expected, actual);
            }
            other => panic!("Expected ChecksumMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_restore_handles_missing_backup_file() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_fake_db(&db_dir, "core.db");

        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();

        // Delete the backed-up file
        let backed_up_path = Path::new(&manifest.backup_dir).join("core.db");
        fs::remove_file(&backed_up_path).unwrap();

        let manifest_path = Path::new(&manifest.backup_dir).join("manifest.json");
        let report = mgr
            .restore_from(manifest_path.to_str().unwrap())
            .unwrap();

        assert!(!report.all_ok);
        assert_eq!(report.results[0].status, RestoreFileStatus::NotFound);
    }

    #[test]
    fn test_restore_invalid_manifest() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();
        fs::create_dir_all(&backup_dir).unwrap();

        let bad_manifest = backup_dir.join("bad_manifest.json");
        fs::write(&bad_manifest, "not valid json!!!").unwrap();

        let mgr = make_manager(&db_dir, &backup_dir);
        let result = mgr.restore_from(bad_manifest.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_restore_nonexistent_manifest() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        let mgr = make_manager(&db_dir, &backup_dir);
        let result = mgr.restore_from("/nonexistent/path/manifest.json");
        assert!(result.is_err());
    }

    // -- Health Check Tests -------------------------------------------------

    #[test]
    fn test_health_check_all_ok() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        fs::create_dir_all(&db_dir).unwrap();

        create_all_dbs(&db_dir);

        let mgr = make_manager(&db_dir, &tmp.path().join("backups"));
        let results = mgr.check_db_integrity();

        assert_eq!(results.len(), 6);
        for r in &results {
            assert_eq!(r.status, DbHealthStatus::Ok, "DB {} not ok", r.db_name);
            assert!(r.size_bytes > 0);
        }
    }

    #[test]
    fn test_health_check_missing_db() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        fs::create_dir_all(&db_dir).unwrap();

        // Only create 1 DB
        create_fake_db(&db_dir, "core.db");

        let mgr = make_manager(&db_dir, &tmp.path().join("backups"));
        let results = mgr.check_db_integrity();

        assert_eq!(results.len(), 6);

        let core = results.iter().find(|r| r.db_name == "core.db").unwrap();
        assert_eq!(core.status, DbHealthStatus::Ok);

        let costs = results.iter().find(|r| r.db_name == "costs.db").unwrap();
        assert_eq!(costs.status, DbHealthStatus::Missing);
        assert_eq!(costs.size_bytes, 0);
    }

    #[test]
    fn test_health_check_corrupted_db() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        fs::create_dir_all(&db_dir).unwrap();

        // Write garbage data as a "database"
        let corrupt_path = db_dir.join("core.db");
        fs::write(&corrupt_path, b"this is not a sqlite database at all!!!").unwrap();

        let mgr = make_manager(&db_dir, &tmp.path().join("backups"));
        let results = mgr.check_db_integrity();

        let core = results.iter().find(|r| r.db_name == "core.db").unwrap();
        match &core.status {
            DbHealthStatus::Ok => panic!("Corrupted DB should not be Ok"),
            DbHealthStatus::Missing => panic!("File exists, should not be Missing"),
            DbHealthStatus::Corrupted(_) | DbHealthStatus::OpenError(_) => {
                // Expected — either open fails or integrity_check fails
            }
        }
    }

    // -- SHA-256 Tests ------------------------------------------------------

    #[test]
    fn test_sha256_file_consistency() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test.bin");
        fs::write(&file_path, b"deterministic content").unwrap();

        let hash1 = sha256_file(&file_path).unwrap();
        let hash2 = sha256_file(&file_path).unwrap();
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }

    #[test]
    fn test_sha256_different_content_different_hash() {
        let tmp = TempDir::new().unwrap();
        let f1 = tmp.path().join("a.bin");
        let f2 = tmp.path().join("b.bin");
        fs::write(&f1, b"content A").unwrap();
        fs::write(&f2, b"content B").unwrap();

        let h1 = sha256_file(&f1).unwrap();
        let h2 = sha256_file(&f2).unwrap();
        assert_ne!(h1, h2);
    }

    // -- List & Storage Summary Tests ---------------------------------------

    #[test]
    fn test_list_backups_empty() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        let mgr = make_manager(&db_dir, &backup_dir);
        let list = mgr.list_backups().unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_list_backups_after_creating() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_fake_db(&db_dir, "core.db");

        let mgr = make_manager(&db_dir, &backup_dir);
        mgr.backup_all().unwrap();

        let list = mgr.list_backups().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].files.len(), 1);
        assert_eq!(list[0].files[0].filename, "core.db");
    }

    #[test]
    fn test_storage_summary() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("dbs");
        let backup_dir = tmp.path().join("backups");
        fs::create_dir_all(&db_dir).unwrap();

        create_all_dbs(&db_dir);

        let mgr = make_manager(&db_dir, &backup_dir);
        mgr.backup_all().unwrap();

        let summary = mgr.storage_summary().unwrap();
        assert_eq!(summary.backup_count, 1);
        assert!(summary.total_bytes > 0);
        assert_eq!(summary.max_backups, DEFAULT_MAX_BACKUPS);
    }

    // -- Builder / Config Tests ---------------------------------------------

    #[test]
    fn test_with_max_backups() {
        let tmp = TempDir::new().unwrap();
        let mgr = BackupManager::new(tmp.path(), tmp.path()).with_max_backups(3);
        assert_eq!(mgr.max_backups, 3);
    }

    #[test]
    fn test_with_schedule() {
        let tmp = TempDir::new().unwrap();
        let schedule = BackupSchedule {
            cron_expr: "0 4 * * 0".to_string(),
        };
        let mgr = BackupManager::new(tmp.path(), tmp.path()).with_schedule(schedule);
        assert_eq!(mgr.schedule.cron_expr, "0 4 * * 0");
    }

    // -- Round-trip Test ----------------------------------------------------

    #[test]
    fn test_full_backup_restore_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let db_dir = tmp.path().join("original");
        let backup_dir = tmp.path().join("backups");
        let restore_dir = tmp.path().join("restored");
        fs::create_dir_all(&db_dir).unwrap();
        fs::create_dir_all(&restore_dir).unwrap();

        // Create DBs with distinctive data
        for (i, &name) in MANAGED_DBS.iter().enumerate() {
            let path = db_dir.join(name);
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE info (id INTEGER PRIMARY KEY, val TEXT);
                 INSERT INTO info (val) VALUES ('db-{}-data-{}');",
                name, i
            ))
            .unwrap();
        }

        // Backup
        let mgr = make_manager(&db_dir, &backup_dir);
        let manifest = mgr.backup_all().unwrap();
        assert_eq!(manifest.files.len(), 6);

        // Verify integrity of originals
        let health = mgr.check_db_integrity();
        assert!(health.iter().all(|h| h.status == DbHealthStatus::Ok));

        // Restore to different dir
        let restore_mgr = make_manager(&restore_dir, &backup_dir);
        let manifest_path = Path::new(&manifest.backup_dir).join("manifest.json");
        let report = restore_mgr
            .restore_from(manifest_path.to_str().unwrap())
            .unwrap();
        assert!(report.all_ok);

        // Verify restored DBs contain the right data
        for (i, &name) in MANAGED_DBS.iter().enumerate() {
            let path = restore_dir.join(name);
            let conn = Connection::open(&path).unwrap();
            let val: String = conn
                .query_row("SELECT val FROM info WHERE id = 1", [], |row| row.get(0))
                .unwrap();
            assert_eq!(val, format!("db-{}-data-{}", name, i));
        }

        // Verify restored DB integrity
        let restored_health = restore_mgr.check_db_integrity();
        assert!(restored_health
            .iter()
            .all(|h| h.status == DbHealthStatus::Ok));
    }

    #[test]
    fn test_managed_dbs_constant() {
        assert_eq!(MANAGED_DBS.len(), 6);
        assert!(MANAGED_DBS.contains(&"core.db"));
        assert!(MANAGED_DBS.contains(&"costs.db"));
        assert!(MANAGED_DBS.contains(&"memory.db"));
        assert!(MANAGED_DBS.contains(&"knowledge.db"));
        assert!(MANAGED_DBS.contains(&"revenue.db"));
        assert!(MANAGED_DBS.contains(&"trajectories.db"));
    }
}
