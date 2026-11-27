//! Sending a level package (opaque bytes, see `mw_level::LevelBundle::to_package`) from the
//! server to a client: offer → (request) → chunks → ready. Pure state, no sockets.
//!
//! The client may already have the package in its [`LevelCache`] (same hash): it then skips
//! the download and answers `LevelReady` directly.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bytes per [`crate::ServerMessage::LevelChunk`].
pub const CHUNK_SIZE: usize = 16 * 1024;

/// Largest accepted package. Kept well under the reliable channel's memory (5 MiB) so a
/// whole level can be queued at once.
pub const MAX_PACKAGE_BYTES: usize = 4 * 1024 * 1024;

/// SHA-256 of a package: identity for the cache and integrity check after reassembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LevelHash(pub [u8; 32]);

impl LevelHash {
    pub fn of(bytes: &[u8]) -> Self {
        let digest = Sha256::digest(bytes);
        let mut out = [0u8; 32];
        out.copy_from_slice(&digest);
        Self(out)
    }

    /// Lowercase hexadecimal, for file names and logs.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// What the server announces before sending anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LevelOffer {
    /// Increases with every level the server switches to; requests and acknowledgements
    /// carry it so messages about an older level are ignored.
    pub transfer: u32,
    pub hash: LevelHash,
    pub size: u32,
    pub chunks: u32,
}

impl LevelOffer {
    /// The offer for `package` as transfer number `transfer`.
    pub fn new(transfer: u32, package: &[u8]) -> Self {
        Self {
            transfer,
            hash: LevelHash::of(package),
            size: package.len() as u32,
            chunks: package.len().div_ceil(CHUNK_SIZE) as u32,
        }
    }
}

/// Splits a package into chunks of at most [`CHUNK_SIZE`] (one empty chunk for no data).
pub fn chunks(package: &[u8]) -> Vec<&[u8]> {
    if package.is_empty() {
        return vec![package];
    }
    package.chunks(CHUNK_SIZE).collect()
}

/// Why a transfer failed on the client.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransferError {
    #[error("offered level is too large ({size} bytes, max {MAX_PACKAGE_BYTES})")]
    TooLarge { size: u32 },
    #[error("offer is inconsistent ({chunks} chunks for {size} bytes)")]
    BadOffer { size: u32, chunks: u32 },
    #[error("chunk {index} is invalid (out of range or wrong size)")]
    BadChunk { index: u32 },
    #[error("reassembled level does not match its hash")]
    HashMismatch,
}

/// Rebuilds a package from chunks received in any order.
#[derive(Debug)]
pub struct Assembler {
    offer: LevelOffer,
    parts: Vec<Option<Vec<u8>>>,
    missing: u32,
}

impl Assembler {
    /// Starts receiving `offer`, refusing offers that are too large or inconsistent.
    pub fn new(offer: LevelOffer) -> Result<Self, TransferError> {
        let size = offer.size as usize;
        if size > MAX_PACKAGE_BYTES {
            return Err(TransferError::TooLarge { size: offer.size });
        }
        let expected = size.div_ceil(CHUNK_SIZE).max(1) as u32;
        let chunks_ok = offer.chunks == expected || (size == 0 && offer.chunks == 0);
        if !chunks_ok {
            return Err(TransferError::BadOffer {
                size: offer.size,
                chunks: offer.chunks,
            });
        }
        let count = offer.chunks.max(1);
        Ok(Self {
            offer,
            parts: vec![None; count as usize],
            missing: count,
        })
    }

    pub fn offer(&self) -> &LevelOffer {
        &self.offer
    }

    /// Fraction received, for a progress bar.
    pub fn progress(&self) -> f32 {
        1.0 - self.missing as f32 / self.parts.len() as f32
    }

    /// Adds chunk `index`. Returns the verified package once every chunk is there.
    pub fn add(&mut self, index: u32, data: Vec<u8>) -> Result<Option<Vec<u8>>, TransferError> {
        let i = index as usize;
        let last = self.parts.len() - 1;
        let size = self.offer.size as usize;
        // Every chunk is full except the last one (offers are checked consistent in `new`).
        let expected_len = if i == last {
            size - last * CHUNK_SIZE
        } else {
            CHUNK_SIZE
        };
        if i > last || data.len() != expected_len {
            return Err(TransferError::BadChunk { index });
        }
        if self.parts[i].is_none() {
            self.parts[i] = Some(data);
            self.missing -= 1;
        }
        if self.missing > 0 {
            return Ok(None);
        }
        let package: Vec<u8> = self.parts.iter().flatten().flatten().copied().collect();
        if LevelHash::of(&package) != self.offer.hash {
            return Err(TransferError::HashMismatch);
        }
        Ok(Some(package))
    }
}

/// Optional on-disk cache of packages, keyed by hash (`<dir>/<hash>.mwlevel`). Without a
/// directory it caches nothing. Reads are verified: a corrupted file is a miss.
#[derive(Debug, Clone, Default)]
pub struct LevelCache {
    dir: Option<PathBuf>,
}

impl LevelCache {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    fn path(&self, hash: &LevelHash) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(format!("{}.mwlevel", hash.to_hex())))
    }

    /// The cached package with this hash, if present and intact.
    pub fn get(&self, hash: &LevelHash) -> Option<Vec<u8>> {
        let bytes = std::fs::read(self.path(hash)?).ok()?;
        (LevelHash::of(&bytes) == *hash).then_some(bytes)
    }

    /// Stores `package` (best effort: a cache failure never breaks a game).
    pub fn put(&self, package: &[u8]) -> std::io::Result<()> {
        let Some(path) = self.path(&LevelHash::of(package)) else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, package)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 7 % 251) as u8).collect()
    }

    fn send(
        data: &[u8],
        order: impl Fn(usize) -> Vec<usize>,
    ) -> Result<Option<Vec<u8>>, TransferError> {
        let offer = LevelOffer::new(1, data);
        let mut assembler = Assembler::new(offer)?;
        let parts = chunks(data);
        let mut result = None;
        for i in order(parts.len()) {
            result = assembler.add(i as u32, parts[i].to_vec())?;
        }
        Ok(result)
    }

    #[test]
    fn reassembles_in_any_order() {
        for len in [0, 1, CHUNK_SIZE, CHUNK_SIZE + 1, 3 * CHUNK_SIZE + 17] {
            let data = package(len);
            assert_eq!(
                send(&data, |n| (0..n).collect()).unwrap(),
                Some(data.clone()),
                "len {len}"
            );
            assert_eq!(
                send(&data, |n| (0..n).rev().collect()).unwrap(),
                Some(data.clone()),
                "len {len} reversed"
            );
        }
    }

    #[test]
    fn incomplete_until_the_last_chunk_and_duplicates_are_harmless() {
        let data = package(2 * CHUNK_SIZE + 5);
        let mut a = Assembler::new(LevelOffer::new(1, &data)).unwrap();
        let parts = chunks(&data);
        assert_eq!(a.add(0, parts[0].to_vec()).unwrap(), None);
        assert_eq!(a.add(0, parts[0].to_vec()).unwrap(), None);
        assert!((a.progress() - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(a.add(2, parts[2].to_vec()).unwrap(), None);
        assert_eq!(a.add(1, parts[1].to_vec()).unwrap(), Some(data));
    }

    #[test]
    fn rejects_bad_offers_chunks_and_corruption() {
        let data = package(CHUNK_SIZE + 10);
        let offer = LevelOffer::new(1, &data);
        assert!(matches!(
            Assembler::new(LevelOffer {
                size: (MAX_PACKAGE_BYTES + 1) as u32,
                ..offer
            }),
            Err(TransferError::TooLarge { .. })
        ));
        assert!(matches!(
            Assembler::new(LevelOffer { chunks: 7, ..offer }),
            Err(TransferError::BadOffer { .. })
        ));
        let mut a = Assembler::new(offer).unwrap();
        assert_eq!(
            a.add(5, vec![0; 10]),
            Err(TransferError::BadChunk { index: 5 })
        );
        assert_eq!(
            a.add(1, vec![0; 11]),
            Err(TransferError::BadChunk { index: 1 })
        );
        let parts = chunks(&data);
        a.add(0, parts[0].to_vec()).unwrap();
        let mut corrupted = parts[1].to_vec();
        corrupted[0] ^= 1;
        assert_eq!(a.add(1, corrupted), Err(TransferError::HashMismatch));
    }

    #[test]
    fn cache_roundtrip_and_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let cache = LevelCache::new(Some(dir.path().join("levels")));
        let data = package(1000);
        let hash = LevelHash::of(&data);
        assert_eq!(cache.get(&hash), None);
        cache.put(&data).unwrap();
        assert_eq!(cache.get(&hash), Some(data.clone()));
        // A file whose content no longer matches its name is ignored.
        std::fs::write(
            dir.path()
                .join("levels")
                .join(format!("{}.mwlevel", hash.to_hex())),
            b"x",
        )
        .unwrap();
        assert_eq!(cache.get(&hash), None);
        // No directory: nothing is cached, nothing fails.
        let none = LevelCache::default();
        none.put(&data).unwrap();
        assert_eq!(none.get(&hash), None);
    }

    #[test]
    fn hash_hex_is_stable() {
        assert_eq!(
            LevelHash::of(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
