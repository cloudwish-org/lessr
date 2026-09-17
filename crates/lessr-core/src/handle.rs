//! The handle store: nothing is deleted, only deferred.
//!
//! Every cut leaves a handle (invariant 3). A handle is a short hex id derived
//! from the blake3 hash of the removed bytes; `lessr show <id>` gives the bytes
//! back. The store is an append-only file for the session with an in-memory
//! index, so a stage writes once and never seeks.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use bytes::Bytes;

use crate::error::{Error, Result};

/// The shortest id we print. 4 hex characters is what fits in a log line
/// without becoming noise.
const MIN_HEX: usize = 4;
/// The longest id we are willing to grow to when ids collide.
const MAX_HEX: usize = 16;

/// A short, printable reference to removed bytes.
///
/// Copy and allocation-free: handles are created on every cut, and a cut can
/// happen inside a filter loop.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleId {
    hex: [u8; MAX_HEX],
    len: u8,
}

impl HandleId {
    /// Take the first `len` hex characters of a hash.
    fn from_hash(hash: &[u8; 32], len: usize) -> Self {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut hex = [b'0'; MAX_HEX];
        for i in 0..len {
            let byte = hash[i / 2];
            let nibble = if i % 2 == 0 { byte >> 4 } else { byte & 0x0f };
            hex[i] = HEX[nibble as usize];
        }
        Self {
            hex,
            len: len as u8,
        }
    }

    /// Read an id back from what a receipt or a log line printed.
    pub fn parse(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if !(MIN_HEX..=MAX_HEX).contains(&bytes.len()) {
            return None;
        }
        if !bytes.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut hex = [b'0'; MAX_HEX];
        for (slot, b) in hex.iter_mut().zip(bytes) {
            *slot = b.to_ascii_lowercase();
        }
        Some(Self {
            hex,
            len: bytes.len() as u8,
        })
    }

    /// The id as text.
    pub fn as_str(&self) -> &str {
        // The buffer only ever holds ASCII hex, written by `from_hash` or
        // validated by `parse`.
        std::str::from_utf8(&self.hex[..self.len as usize]).unwrap_or("????")
    }
}

impl std::fmt::Display for HandleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::fmt::Debug for HandleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HandleId({})", self.as_str())
    }
}

/// Where the bytes behind a handle live.
#[derive(Clone, Debug)]
enum Location {
    /// In the session file, at this offset and length.
    File { offset: u64, len: u32 },
    /// In memory, for a store with no file behind it.
    Memory(Bytes),
}

#[derive(Clone, Debug)]
struct Entry {
    hash: [u8; 32],
    location: Location,
}

/// An append-only store of everything the stages cut.
pub struct HandleStore {
    file: Option<File>,
    /// Bytes written so far, so we know the offset of the next record without
    /// asking the filesystem.
    end: u64,
    index: HashMap<HandleId, Entry, ahash::RandomState>,
    by_hash: HashMap<[u8; 32], HandleId, ahash::RandomState>,
}

impl HandleStore {
    /// A store that keeps everything in memory. Used by tests, and by any path
    /// that has no config directory to write to.
    pub fn in_memory() -> Self {
        Self {
            file: None,
            end: 0,
            index: HashMap::default(),
            by_hash: HashMap::default(),
        }
    }

    /// Open (or create) the session file and replay its index.
    ///
    /// Replaying is what lets `lessr show <id>` work from a different process
    /// than the one that made the cut.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::HandleStore)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)
            .map_err(Error::HandleStore)?;

        let mut store = Self {
            file: None,
            end: 0,
            index: HashMap::default(),
            by_hash: HashMap::default(),
        };
        store.replay(&mut file)?;
        store.file = Some(file);
        Ok(store)
    }

    /// Walk the file once, rebuilding the index without reading any content.
    fn replay(&mut self, file: &mut File) -> Result<()> {
        let size = file.metadata().map_err(Error::HandleStore)?.len();
        file.seek(SeekFrom::Start(0)).map_err(Error::HandleStore)?;
        let mut offset = 0u64;

        while offset < size {
            let mut header = [0u8; 2];
            if file.read_exact(&mut header).is_err() {
                break;
            }
            let id_len = header[0] as usize;
            let stage_len = header[1] as usize;
            if !(MIN_HEX..=MAX_HEX).contains(&id_len) {
                return Err(Error::CorruptHandle(format!("at offset {offset}")));
            }

            let mut rest = vec![0u8; id_len + stage_len + 32 + 4];
            file.read_exact(&mut rest)
                .map_err(|_| Error::CorruptHandle(format!("at offset {offset}")))?;

            let id = HandleId::parse(
                std::str::from_utf8(&rest[..id_len])
                    .map_err(|_| Error::CorruptHandle(format!("at offset {offset}")))?,
            )
            .ok_or_else(|| Error::CorruptHandle(format!("at offset {offset}")))?;

            let hash_at = id_len + stage_len;
            let mut hash = [0u8; 32];
            hash.copy_from_slice(&rest[hash_at..hash_at + 32]);
            let len = u32::from_le_bytes([
                rest[hash_at + 32],
                rest[hash_at + 33],
                rest[hash_at + 34],
                rest[hash_at + 35],
            ]);

            let content_at = offset + 2 + rest.len() as u64;
            self.index.insert(
                id,
                Entry {
                    hash,
                    location: Location::File {
                        offset: content_at,
                        len,
                    },
                },
            );
            self.by_hash.insert(hash, id);

            offset = content_at + u64::from(len);
            file.seek(SeekFrom::Start(offset))
                .map_err(Error::HandleStore)?;
        }

        self.end = offset;
        Ok(())
    }

    /// Store `content` and return the handle that brings it back.
    ///
    /// Storing the same bytes twice returns the same id and writes nothing: a
    /// file read and re-read in one session costs one record.
    pub fn put(&mut self, stage: &str, content: &[u8]) -> Result<HandleId> {
        let hash: [u8; 32] = *blake3::hash(content).as_bytes();
        if let Some(existing) = self.by_hash.get(&hash) {
            return Ok(*existing);
        }

        let id = self.free_id(&hash)?;
        let location = match self.file.as_mut() {
            Some(file) => {
                let stage_bytes = stage.as_bytes();
                let stage_len = stage_bytes.len().min(u8::MAX as usize);
                let id_str = id.as_str().as_bytes();
                let len = u32::try_from(content.len()).unwrap_or(u32::MAX);

                let mut record =
                    Vec::with_capacity(2 + id_str.len() + stage_len + 36 + content.len());
                record.push(id_str.len() as u8);
                record.push(stage_len as u8);
                record.extend_from_slice(id_str);
                record.extend_from_slice(&stage_bytes[..stage_len]);
                record.extend_from_slice(&hash);
                record.extend_from_slice(&len.to_le_bytes());
                let content_at = self.end + (record.len() as u64);
                record.extend_from_slice(&content[..len as usize]);

                file.write_all(&record).map_err(Error::HandleStore)?;
                self.end += record.len() as u64;
                Location::File {
                    offset: content_at,
                    len,
                }
            }
            None => Location::Memory(Bytes::copy_from_slice(content)),
        };

        self.index.insert(id, Entry { hash, location });
        self.by_hash.insert(hash, id);
        Ok(id)
    }

    /// The shortest id not already taken by different content.
    fn free_id(&self, hash: &[u8; 32]) -> Result<HandleId> {
        for len in MIN_HEX..=MAX_HEX {
            let id = HandleId::from_hash(hash, len);
            match self.index.get(&id) {
                None => return Ok(id),
                Some(entry) if entry.hash == *hash => return Ok(id),
                Some(_) => continue,
            }
        }
        Err(Error::HandleCollision)
    }

    /// The original bytes behind a handle, if this store has them.
    pub fn get(&mut self, id: HandleId) -> Result<Option<Bytes>> {
        let Some(entry) = self.index.get(&id).cloned() else {
            return Ok(None);
        };
        match entry.location {
            Location::Memory(bytes) => Ok(Some(bytes)),
            Location::File { offset, len } => {
                let file = self
                    .file
                    .as_mut()
                    .ok_or(Error::CorruptHandle(id.to_string()))?;
                file.seek(SeekFrom::Start(offset))
                    .map_err(Error::HandleStore)?;
                let mut buf = vec![0u8; len as usize];
                file.read_exact(&mut buf)
                    .map_err(|_| Error::CorruptHandle(id.to_string()))?;
                Ok(Some(Bytes::from(buf)))
            }
        }
    }

    /// How many distinct handles this session has made.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether nothing has been cut yet.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

impl Default for HandleStore {
    fn default() -> Self {
        Self::in_memory()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_returns_the_original_bytes() {
        let mut store = HandleStore::in_memory();
        let id = store.put("gate", b"the original bytes").unwrap();
        assert_eq!(
            store.get(id).unwrap().as_deref(),
            Some(&b"the original bytes"[..])
        );
    }

    #[test]
    fn ids_are_four_hex_characters_by_default() {
        let mut store = HandleStore::in_memory();
        let id = store.put("gate", b"anything").unwrap();
        assert_eq!(id.as_str().len(), 4);
        assert!(id.as_str().bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[test]
    fn the_same_bytes_twice_is_one_handle() {
        let mut store = HandleStore::in_memory();
        let a = store.put("gate", b"same").unwrap();
        let b = store.put("dedup", b"same").unwrap();
        assert_eq!(a, b);
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn an_unknown_handle_is_none_not_an_error() {
        let mut store = HandleStore::in_memory();
        let id = HandleId::parse("beef").unwrap();
        assert!(store.get(id).unwrap().is_none());
    }

    #[test]
    fn parse_rejects_what_is_not_a_handle() {
        assert!(HandleId::parse("abc").is_none(), "too short");
        assert!(HandleId::parse("zzzz").is_none(), "not hex");
        assert!(HandleId::parse("00112233445566778").is_none(), "too long");
        assert_eq!(HandleId::parse("DEAD").unwrap().as_str(), "dead");
    }

    #[test]
    fn a_colliding_id_grows_until_it_is_free() {
        let mut store = HandleStore::in_memory();
        let hash = [0xabu8; 32];
        let first = HandleId::from_hash(&hash, 4);
        store.index.insert(
            first,
            Entry {
                hash: [0x00; 32],
                location: Location::Memory(Bytes::new()),
            },
        );
        let next = store.free_id(&hash).unwrap();
        assert_eq!(next.as_str().len(), 5);
    }

    #[test]
    fn handles_survive_the_process_that_made_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session").join("handles.bin");

        let (one, two) = {
            let mut store = HandleStore::open(&path).unwrap();
            (
                store.put("gate", b"cut by the gate").unwrap(),
                store.put("trap", b"a lockfile, trapped").unwrap(),
            )
        };

        let mut reopened = HandleStore::open(&path).unwrap();
        assert_eq!(reopened.len(), 2);
        assert_eq!(
            reopened.get(one).unwrap().as_deref(),
            Some(&b"cut by the gate"[..])
        );
        assert_eq!(
            reopened.get(two).unwrap().as_deref(),
            Some(&b"a lockfile, trapped"[..])
        );
    }

    #[test]
    fn a_reopened_store_keeps_appending() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("handles.bin");

        let first = HandleStore::open(&path)
            .unwrap()
            .put("gate", b"one")
            .unwrap();
        let second = {
            let mut store = HandleStore::open(&path).unwrap();
            store.put("gate", b"two").unwrap()
        };

        let mut store = HandleStore::open(&path).unwrap();
        assert_eq!(store.len(), 2);
        assert_eq!(store.get(first).unwrap().as_deref(), Some(&b"one"[..]));
        assert_eq!(store.get(second).unwrap().as_deref(), Some(&b"two"[..]));
    }
}
