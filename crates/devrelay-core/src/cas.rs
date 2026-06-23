//! Content-addressed storage for large sidecar data.
//!
//! This module implements the local CAS invariants independently from any
//! network transport: chunk hashes are canonical, uploads and downloads verify
//! bytes, writes are atomic, manifests are content-derived, and reachability
//! roots can pin manifests for later GC.

use crate::{DevRelayError, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const CAS_SCHEMA_VERSION: u32 = 1;
pub const CAS_HASH_PREFIX: &str = "b3_";
const BLAKE3_HEX_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CasChunkHash(String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasChunkRecord {
    pub hash: CasChunkHash,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasUploadResult {
    pub chunk: CasChunkRecord,
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasManifestChunk {
    pub hash: CasChunkHash,
    pub offset_bytes: u64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasManifest {
    pub schema_version: u32,
    pub manifest_id: String,
    pub total_bytes: u64,
    pub chunks: Vec<CasManifestChunk>,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CasReachabilityRoot {
    pub root_id: String,
    pub manifest_id: String,
    pub created_at_unix_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct CasStore {
    root: PathBuf,
}

impl CasChunkHash {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(format!("{CAS_HASH_PREFIX}{}", blake3::hash(bytes).to_hex()))
    }

    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        validate_hash_format(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn hex(&self) -> &str {
        &self.0[CAS_HASH_PREFIX.len()..]
    }
}

impl Serialize for CasChunkHash {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for CasChunkHash {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl CasManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CAS_SCHEMA_VERSION {
            return Err(DevRelayError::Config(format!(
                "unsupported CAS manifest schema {}, expected {}",
                self.schema_version, CAS_SCHEMA_VERSION
            )));
        }
        validate_hash_format(&self.manifest_id)?;
        let expected_id = calculate_manifest_id(&self.chunks);
        if self.manifest_id != expected_id {
            return Err(DevRelayError::Config(format!(
                "CAS manifest id {} does not match content hash {}",
                self.manifest_id, expected_id
            )));
        }

        let mut offset = 0_u64;
        for chunk in &self.chunks {
            if chunk.offset_bytes != offset {
                return Err(DevRelayError::Config(format!(
                    "CAS manifest chunk offset {} does not match expected {}",
                    chunk.offset_bytes, offset
                )));
            }
            offset = offset.saturating_add(chunk.size_bytes);
        }
        if self.total_bytes != offset {
            return Err(DevRelayError::Config(format!(
                "CAS manifest total_bytes {} does not match chunks {}",
                self.total_bytes, offset
            )));
        }
        Ok(())
    }
}

impl CasStore {
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { root: root.into() };
        for dir in [
            store.chunks_root(),
            store.manifests_root(),
            store.reachability_roots_root(),
            store.tmp_root(),
        ] {
            fs::create_dir_all(dir)?;
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn upload_chunk(
        &self,
        bytes: &[u8],
        expected_hash: &CasChunkHash,
    ) -> Result<CasUploadResult> {
        let actual_hash = CasChunkHash::from_bytes(bytes);
        if &actual_hash != expected_hash {
            return Err(DevRelayError::Config(format!(
                "CAS chunk upload hash mismatch: expected {}, got {}",
                expected_hash.as_str(),
                actual_hash.as_str()
            )));
        }

        let path = self.chunk_path(expected_hash);
        if path.exists() {
            self.verify_chunk_file(expected_hash)?;
            return Ok(CasUploadResult {
                chunk: CasChunkRecord {
                    hash: expected_hash.clone(),
                    size_bytes: bytes.len() as u64,
                },
                stored: false,
            });
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.tmp_path("chunk");
        let write_result = (|| -> Result<()> {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        write_result?;

        Ok(CasUploadResult {
            chunk: CasChunkRecord {
                hash: expected_hash.clone(),
                size_bytes: bytes.len() as u64,
            },
            stored: true,
        })
    }

    pub fn download_chunk(&self, hash: &CasChunkHash) -> Result<Vec<u8>> {
        let bytes = fs::read(self.chunk_path(hash))?;
        let actual = CasChunkHash::from_bytes(&bytes);
        if &actual != hash {
            return Err(DevRelayError::Config(format!(
                "CAS chunk {} failed download verification; got {}",
                hash.as_str(),
                actual.as_str()
            )));
        }
        Ok(bytes)
    }

    pub fn missing_chunks(&self, hashes: &[CasChunkHash]) -> Vec<CasChunkHash> {
        hashes
            .iter()
            .filter(|hash| self.verify_chunk_file(hash).is_err())
            .cloned()
            .collect()
    }

    pub fn create_manifest(&self, chunk_hashes: &[CasChunkHash]) -> Result<CasManifest> {
        let mut chunks = Vec::with_capacity(chunk_hashes.len());
        let mut offset = 0_u64;
        for hash in chunk_hashes {
            let size_bytes = self.verify_chunk_file(hash)?;
            chunks.push(CasManifestChunk {
                hash: hash.clone(),
                offset_bytes: offset,
                size_bytes,
            });
            offset = offset.saturating_add(size_bytes);
        }
        let manifest = CasManifest {
            schema_version: CAS_SCHEMA_VERSION,
            manifest_id: calculate_manifest_id(&chunks),
            total_bytes: offset,
            chunks,
            created_at_unix_seconds: unix_now_seconds(),
        };
        manifest.validate()?;
        self.write_json_atomically(&self.manifest_path(&manifest.manifest_id)?, &manifest)?;
        Ok(manifest)
    }

    pub fn fetch_manifest(&self, manifest_id: &str) -> Result<CasManifest> {
        validate_hash_format(manifest_id)?;
        let raw = fs::read_to_string(self.manifest_path(manifest_id)?)?;
        let manifest: CasManifest = serde_json::from_str(&raw)?;
        manifest.validate()?;
        if manifest.manifest_id != manifest_id {
            return Err(DevRelayError::Config(format!(
                "CAS manifest file {} contained {}",
                manifest_id, manifest.manifest_id
            )));
        }
        Ok(manifest)
    }

    pub fn add_reachability_root(
        &self,
        root_id: &str,
        manifest_id: &str,
    ) -> Result<CasReachabilityRoot> {
        validate_root_id(root_id)?;
        self.fetch_manifest(manifest_id)?;
        let root = CasReachabilityRoot {
            root_id: root_id.to_string(),
            manifest_id: manifest_id.to_string(),
            created_at_unix_seconds: unix_now_seconds(),
        };
        self.write_json_atomically(&self.reachability_root_path(root_id)?, &root)?;
        Ok(root)
    }

    pub fn fetch_reachability_root(&self, root_id: &str) -> Result<CasReachabilityRoot> {
        validate_root_id(root_id)?;
        let raw = fs::read_to_string(self.reachability_root_path(root_id)?)?;
        let root: CasReachabilityRoot = serde_json::from_str(&raw)?;
        validate_root_id(&root.root_id)?;
        validate_hash_format(&root.manifest_id)?;
        Ok(root)
    }

    fn verify_chunk_file(&self, hash: &CasChunkHash) -> Result<u64> {
        let bytes = self.download_chunk(hash)?;
        Ok(bytes.len() as u64)
    }

    fn chunks_root(&self) -> PathBuf {
        self.root.join("chunks").join("b3")
    }

    fn manifests_root(&self) -> PathBuf {
        self.root.join("manifests")
    }

    fn reachability_roots_root(&self) -> PathBuf {
        self.root.join("roots")
    }

    fn tmp_root(&self) -> PathBuf {
        self.root.join("tmp")
    }

    fn chunk_path(&self, hash: &CasChunkHash) -> PathBuf {
        let hex = hash.hex();
        self.chunks_root()
            .join(&hex[0..2])
            .join(format!("{}.chunk", &hex[2..]))
    }

    fn manifest_path(&self, manifest_id: &str) -> Result<PathBuf> {
        validate_hash_format(manifest_id)?;
        Ok(self.manifests_root().join(format!("{manifest_id}.json")))
    }

    fn reachability_root_path(&self, root_id: &str) -> Result<PathBuf> {
        validate_root_id(root_id)?;
        Ok(self
            .reachability_roots_root()
            .join(format!("{root_id}.json")))
    }

    fn write_json_atomically<T: Serialize>(&self, path: &Path, value: &T) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.tmp_path("json");
        let write_result = (|| -> Result<()> {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
            serde_json::to_writer_pretty(&mut file, value)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&tmp, path)?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        write_result
    }

    fn tmp_path(&self, label: &str) -> PathBuf {
        self.tmp_root().join(format!(
            "{label}-{}-{}.tmp",
            std::process::id(),
            unix_nanos()
        ))
    }
}

fn calculate_manifest_id(chunks: &[CasManifestChunk]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"devrelay.cas.manifest.v1");
    for chunk in chunks {
        hasher.update(chunk.hash.as_str().as_bytes());
        hasher.update(&chunk.offset_bytes.to_le_bytes());
        hasher.update(&chunk.size_bytes.to_le_bytes());
    }
    format!("{CAS_HASH_PREFIX}{}", hasher.finalize().to_hex())
}

fn validate_hash_format(value: &str) -> Result<()> {
    let Some(hex) = value.strip_prefix(CAS_HASH_PREFIX) else {
        return Err(DevRelayError::Config(format!(
            "CAS hash {value} must start with {CAS_HASH_PREFIX}"
        )));
    };
    if hex.len() != BLAKE3_HEX_LEN
        || !hex
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(DevRelayError::Config(format!(
            "CAS hash {value} must contain {BLAKE3_HEX_LEN} lowercase hex characters"
        )));
    }
    Ok(())
}

fn validate_root_id(root_id: &str) -> Result<()> {
    if root_id.is_empty()
        || matches!(root_id, "." | "..")
        || root_id.contains('/')
        || root_id.contains('\\')
        || root_id.contains("..")
        || root_id.bytes().any(|byte| byte <= 0x20 || byte == 0x7f)
    {
        return Err(DevRelayError::Config(format!(
            "CAS reachability root id {root_id} is not safe for a file path"
        )));
    }
    Ok(())
}

fn unix_now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cas_hash_format_is_canonical_and_validated() {
        let hash = CasChunkHash::from_bytes(b"hello");

        assert!(hash.as_str().starts_with(CAS_HASH_PREFIX));
        assert_eq!(hash.as_str().len(), CAS_HASH_PREFIX.len() + BLAKE3_HEX_LEN);
        assert_eq!(CasChunkHash::parse(hash.as_str()).unwrap(), hash);
        assert!(CasChunkHash::parse("sha256_bad").is_err());
        assert!(CasChunkHash::parse(format!("{}ABC", CAS_HASH_PREFIX)).is_err());
    }

    #[test]
    fn cas_uploads_downloads_verifies_and_deduplicates_chunks() {
        let temp = tempfile::tempdir().unwrap();
        let store = CasStore::open(temp.path()).unwrap();
        let bytes = b"chunk data";
        let hash = CasChunkHash::from_bytes(bytes);

        let first = store.upload_chunk(bytes, &hash).unwrap();
        assert!(first.stored);
        assert_eq!(first.chunk.size_bytes, bytes.len() as u64);
        assert_eq!(store.download_chunk(&hash).unwrap(), bytes);

        let second = store.upload_chunk(bytes, &hash).unwrap();
        assert!(!second.stored);
        assert!(store.missing_chunks(std::slice::from_ref(&hash)).is_empty());

        let wrong = CasChunkHash::from_bytes(b"other");
        let err = store.upload_chunk(bytes, &wrong).unwrap_err();
        assert!(err.to_string().contains("hash mismatch"));
    }

    #[test]
    fn cas_missing_query_treats_corrupt_chunks_as_missing() {
        let temp = tempfile::tempdir().unwrap();
        let store = CasStore::open(temp.path()).unwrap();
        let hash = CasChunkHash::from_bytes(b"valid");
        store.upload_chunk(b"valid", &hash).unwrap();

        fs::write(store.chunk_path(&hash), b"corrupt").unwrap();

        let missing = store.missing_chunks(std::slice::from_ref(&hash));
        assert_eq!(missing, vec![hash.clone()]);
        let err = store.download_chunk(&hash).unwrap_err();
        assert!(err.to_string().contains("failed download verification"));
    }

    #[test]
    fn cas_manifest_create_fetch_and_reachability_root_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let store = CasStore::open(temp.path()).unwrap();
        let first = CasChunkHash::from_bytes(b"first");
        let second = CasChunkHash::from_bytes(b"second");
        store.upload_chunk(b"first", &first).unwrap();
        store.upload_chunk(b"second", &second).unwrap();

        let manifest = store
            .create_manifest(&[first.clone(), second.clone()])
            .unwrap();

        assert_eq!(manifest.schema_version, CAS_SCHEMA_VERSION);
        assert_eq!(manifest.total_bytes, 11);
        assert_eq!(manifest.chunks[0].offset_bytes, 0);
        assert_eq!(manifest.chunks[1].offset_bytes, 5);
        assert_eq!(manifest.chunks[1].hash, second);

        let fetched = store.fetch_manifest(&manifest.manifest_id).unwrap();
        assert_eq!(fetched.manifest_id, manifest.manifest_id);
        assert_eq!(fetched.chunks, manifest.chunks);

        let root = store
            .add_reachability_root("snapshot-s1_test", &manifest.manifest_id)
            .unwrap();
        assert_eq!(root.manifest_id, manifest.manifest_id);
        let fetched_root = store.fetch_reachability_root("snapshot-s1_test").unwrap();
        assert_eq!(fetched_root.root_id, "snapshot-s1_test");
        assert_eq!(fetched_root.manifest_id, manifest.manifest_id);
    }

    #[test]
    fn cas_manifest_rejects_missing_chunks_and_tampered_content_id() {
        let temp = tempfile::tempdir().unwrap();
        let store = CasStore::open(temp.path()).unwrap();
        let missing = CasChunkHash::from_bytes(b"missing");

        assert!(store.create_manifest(&[missing]).is_err());

        let hash = CasChunkHash::from_bytes(b"present");
        store.upload_chunk(b"present", &hash).unwrap();
        let mut manifest = store.create_manifest(&[hash]).unwrap();
        manifest.total_bytes += 1;
        let err = manifest.validate().unwrap_err();
        assert!(err.to_string().contains("total_bytes"));
    }
}
