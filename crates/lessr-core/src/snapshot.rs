//! The binary snapshot: a resolved [`Config`] the hook path can read without
//! parsing or allocating.
//!
//! The hook runs on every tool call and owes the agent 2 ms, so it parses no
//! TOML and opens no database (technique 7 in `docs/PERFORMANCE.md`). The CLI
//! compiles `config.toml` into this file whenever it changes; the hook maps it
//! (see [`crate::mmap`]) and answers "what mode and level is this stage in,
//! here" by walking fixed-width records and comparing bytes.
//!
//! # Layout
//!
//! Little-endian throughout, `u32` offsets, four regions after a fixed header.
//! Variable-length text lives in one blob and is referenced by offset and
//! length, so every record is a known size and nothing needs alignment.
//!
//! ```text
//! header   56 bytes  magic, version, total length, region table, digest
//! scopes   16 bytes  one per repository, plus one for the global view
//! stages   28 bytes  one per stage per scope, already resolved
//! settings 20 bytes  one per tunable per stage per scope
//! blob     text      names, paths, string values, floor reasons
//! ```
//!
//! # What is not in it
//!
//! The environment layer. A snapshot is a file other processes read, and the
//! environment of whichever shell happened to run `lessr config` has no
//! business in it; the hook applies its own with [`StageView::with_env`].
//!
//! # Reading bytes we did not write
//!
//! The magic, the version and a digest of the body are checked before anything
//! else is believed, and every field is read through a bounds check. A file
//! that fails any of that is refused and the caller falls back to the defaults
//! ([`SnapshotView::read_or_defaults`]): misreading old bytes as new ones would
//! silently change what a mechanism does, which is the one failure a
//! token-saving layer must never have.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use crate::config::{Config, EnvOverrides, Layer};
use crate::error::{Error, Result};
use crate::settings::{Scalar, Value, ValueKind};
use crate::types::{Level, Mode};

/// The first eight bytes of every snapshot.
const MAGIC: [u8; 8] = *b"LESSRCFG";
/// The format version. Bump on any layout change; a reader refuses what it
/// does not recognise rather than guessing.
const VERSION: u16 = 1;

const HEADER_LEN: usize = 56;
const SCOPE_REC: usize = 16;
const STAGE_REC: usize = 28;
const SETTING_REC: usize = 20;

/// The on-disk snapshot format.
///
/// A namespace, not a value: the snapshot itself is a `Vec<u8>` on the way out
/// and a [`SnapshotView`] on the way in.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot;

impl Snapshot {
    /// The magic number a snapshot starts with.
    pub const MAGIC: [u8; 8] = MAGIC;
    /// The format version this build writes and accepts.
    pub const VERSION: u16 = VERSION;

    /// Compile a configuration into snapshot bytes.
    ///
    /// Every stage the configuration names is resolved for the global view and
    /// for each repository named in it, up to [`Layer::RepoStage`] — the
    /// environment is deliberately left out, the safety floor deliberately is
    /// not. A stage named nowhere gets no record, because an absent record and
    /// the compiled-in defaults are the same answer.
    pub fn encode(config: &Config) -> Vec<u8> {
        let mut blob = Blob::default();
        let names = config.stage_names();

        let mut scopes = Vec::with_capacity(SCOPE_REC * (1 + config.repo_scopes().len()));
        let mut stages = Vec::with_capacity(STAGE_REC * names.len());
        let mut settings = Vec::new();

        let mut views: Vec<Option<&Path>> = vec![None];
        views.extend(config.repo_scopes().into_iter().map(Some));

        for scope in views {
            // Lossily, because the blob is text. A repository path that is not
            // valid UTF-8 stops matching itself at lookup time and falls back
            // to the global view: an override can be lost that way, never
            // misapplied to the wrong repository.
            let path = scope.map(Path::to_string_lossy).unwrap_or_default();
            let (path_off, path_len) = blob.push(&path);
            let stage_first = stages.len() / STAGE_REC;

            // The nameless record first: what a stage nobody named resolves
            // to in this scope, which is `[stages.default]` and the repo's
            // own default. Without it a configuration that sets only
            // `[stages.default]` would compile to a snapshot with nothing in
            // it, and every stage would read as the compiled-in default.
            for name in std::iter::once("").chain(names.iter().copied()) {
                let resolved = config.resolve_upto(name, scope, Layer::RepoStage);
                let (name_off, name_len) = blob.push(name);
                let setting_first = settings.len() / SETTING_REC;

                for (key, value) in resolved.settings().iter() {
                    let (key_off, key_len) = blob.push(key);
                    let layer = resolved.setting_layer(key).unwrap_or(Layer::Default);
                    let payload = match value.scalar() {
                        Scalar::Bool(b) => u64::from(b),
                        Scalar::Int(n) => n as u64,
                        Scalar::Str(s) => {
                            let (off, len) = blob.push(s);
                            (u64::from(off) << 32) | u64::from(len)
                        }
                    };
                    settings.extend_from_slice(&key_off.to_le_bytes());
                    settings.extend_from_slice(&key_len.to_le_bytes());
                    settings.push(value.kind().tag());
                    settings.push(layer.tag());
                    settings.extend_from_slice(&0u16.to_le_bytes());
                    settings.extend_from_slice(&payload.to_le_bytes());
                }

                let (reason_off, reason_len) = match resolved.floor_reason() {
                    Some(reason) => blob.push(reason),
                    None => (0, 0),
                };
                let setting_count = settings.len() / SETTING_REC - setting_first;

                stages.extend_from_slice(&name_off.to_le_bytes());
                stages.extend_from_slice(&name_len.to_le_bytes());
                stages.extend_from_slice(&count(setting_first).to_le_bytes());
                stages.extend_from_slice(&count(setting_count).to_le_bytes());
                stages.extend_from_slice(&reason_off.to_le_bytes());
                stages.extend_from_slice(&reason_len.to_le_bytes());
                stages.push(resolved.mode().tag());
                stages.push(resolved.level().tag());
                stages.push(resolved.mode_layer().tag());
                stages.push(resolved.level_layer().tag());
            }

            let stage_count = stages.len() / STAGE_REC - stage_first;
            scopes.extend_from_slice(&path_off.to_le_bytes());
            scopes.extend_from_slice(&path_len.to_le_bytes());
            scopes.extend_from_slice(&count(stage_first).to_le_bytes());
            scopes.extend_from_slice(&count(stage_count).to_le_bytes());
        }

        let scope_off = HEADER_LEN;
        let stage_off = scope_off + scopes.len();
        let setting_off = stage_off + stages.len();
        let blob_off = setting_off + settings.len();
        let total = blob_off + blob.bytes.len();

        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // flags, reserved
        out.extend_from_slice(&count(total).to_le_bytes());
        out.extend_from_slice(&count(scope_off).to_le_bytes());
        out.extend_from_slice(&count(scopes.len() / SCOPE_REC).to_le_bytes());
        out.extend_from_slice(&count(stage_off).to_le_bytes());
        out.extend_from_slice(&count(stages.len() / STAGE_REC).to_le_bytes());
        out.extend_from_slice(&count(setting_off).to_le_bytes());
        out.extend_from_slice(&count(settings.len() / SETTING_REC).to_le_bytes());
        out.extend_from_slice(&count(blob_off).to_le_bytes());
        out.extend_from_slice(&count(blob.bytes.len()).to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // digest, filled in below
        out.extend_from_slice(&scopes);
        out.extend_from_slice(&stages);
        out.extend_from_slice(&settings);
        out.extend_from_slice(&blob.bytes);

        let digest = digest(&out[HEADER_LEN..]);
        out[48..56].copy_from_slice(&digest.to_le_bytes());
        out
    }

    /// Write a snapshot where the hook path will map it.
    ///
    /// Through a temporary file and a rename, so the hook either maps the whole
    /// old snapshot or the whole new one. It also means a mapping already open
    /// keeps pointing at the old inode instead of being truncated underneath
    /// itself, which is what makes [`crate::mmap`]'s `unsafe` defensible.
    pub fn write(path: &Path, config: &Config) -> Result<()> {
        let bytes = Self::encode(config);
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(Error::Snapshot)?;
        }

        let name = path.file_name().ok_or_else(|| {
            Error::Snapshot(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "a snapshot path needs a file name",
            ))
        })?;
        // The pid keeps two `lessr config` runs from writing the same
        // temporary file.
        let mut temp_name = name.to_os_string();
        temp_name.push(format!(".{}.tmp", std::process::id()));
        let temp = path.with_file_name(temp_name);

        let written = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        if let Err(err) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(Error::Snapshot(err));
        }
        if let Err(err) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(Error::Snapshot(err));
        }
        Ok(())
    }

    /// Read snapshot bytes from a file, for a caller that is not mapping it.
    /// The hook path maps instead; see [`crate::mmap::MappedSnapshot`].
    pub fn read(path: &Path) -> Result<Vec<u8>> {
        std::fs::read(path).map_err(Error::Snapshot)
    }
}

/// The blob of text a snapshot's records point into.
#[derive(Default)]
struct Blob {
    bytes: Vec<u8>,
    seen: HashMap<String, (u32, u32), ahash::RandomState>,
}

impl Blob {
    /// Add a string, or find the one already there. Stage names repeat once per
    /// scope and setting keys repeat once per stage, so sharing them is most of
    /// the file.
    fn push(&mut self, text: &str) -> (u32, u32) {
        if let Some(found) = self.seen.get(text) {
            return *found;
        }
        let at = (count(self.bytes.len()), count(text.len()));
        self.bytes.extend_from_slice(text.as_bytes());
        self.seen.insert(text.to_string(), at);
        at
    }
}

/// A count or offset as the format stores it.
///
/// Clamped rather than wrapped. A configuration that overflows a `u32` cannot
/// come from a file a human wrote, and a clamped offset lands outside the blob,
/// where every reader's bounds check turns it into "absent" — which resolves to
/// the default. A wrapped one would point at the wrong bytes.
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The first eight bytes of the blake3 hash of the body.
///
/// Enough to catch a truncated write, a half-copied file or bytes from another
/// format that happen to start with our magic. It is not a signature: a
/// snapshot is a local cache of the user's own config file, not something that
/// crosses a trust boundary.
fn digest(body: &[u8]) -> u64 {
    let hash = blake3::hash(body);
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap_or([0; 8]))
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    bytes
        .get(at..at + 2)
        .and_then(|b| b.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .and_then(|b| b.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    bytes
        .get(at..at + 8)
        .and_then(|b| b.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

/// A read-only view over snapshot bytes.
///
/// Every accessor is a bounds-checked read of a fixed-width record; nothing
/// here allocates, and string settings are borrowed straight out of the mapped
/// file.
#[derive(Clone, Copy, Debug, Default)]
pub struct SnapshotView<'a> {
    scopes: &'a [u8],
    stages: &'a [u8],
    settings: &'a [u8],
    blob: &'a [u8],
}

impl<'a> SnapshotView<'a> {
    /// A view of nothing, which answers every question with the compiled-in
    /// defaults.
    pub const fn empty() -> Self {
        Self {
            scopes: &[],
            stages: &[],
            settings: &[],
            blob: &[],
        }
    }

    /// Validate bytes and return a view of them.
    ///
    /// The magic, the version, the total length and the digest all have to
    /// agree before any record is believed. Use this where the reason matters —
    /// `lessr config` prints it. The hook path uses
    /// [`SnapshotView::read_or_defaults`].
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::SnapshotCorrupt("shorter than its own header"));
        }
        if bytes[..8] != MAGIC {
            return Err(Error::SnapshotMagic);
        }
        let version = u16_at(bytes, 8);
        if version != VERSION {
            return Err(Error::SnapshotVersion {
                found: version,
                expected: VERSION,
            });
        }
        if u32_at(bytes, 12) as usize != bytes.len() {
            return Err(Error::SnapshotCorrupt("length disagrees with the header"));
        }
        if u64_at(bytes, 48) != digest(&bytes[HEADER_LEN..]) {
            return Err(Error::SnapshotCorrupt("digest does not match the body"));
        }

        let scopes = region(bytes, u32_at(bytes, 16), u32_at(bytes, 20), SCOPE_REC)?;
        let stages = region(bytes, u32_at(bytes, 24), u32_at(bytes, 28), STAGE_REC)?;
        let settings = region(bytes, u32_at(bytes, 32), u32_at(bytes, 36), SETTING_REC)?;
        let blob = region(bytes, u32_at(bytes, 40), u32_at(bytes, 44), 1)?;

        Ok(Self {
            scopes,
            stages,
            settings,
            blob,
        })
    }

    /// Validate bytes, falling back to the defaults if they are not a snapshot
    /// this build understands.
    ///
    /// This is the hook path's entry point. A snapshot from a newer Lessr, a
    /// half-written file or something that is not a snapshot at all leaves
    /// every stage on its compiled-in defaults — which is [`Mode::Shadow`], so
    /// the failure mode of an unreadable config is a Lessr that counts and
    /// changes nothing.
    pub fn read_or_defaults(bytes: &'a [u8]) -> Self {
        Self::parse(bytes).unwrap_or_else(|_| Self::empty())
    }

    /// One stage in the global view.
    pub fn stage(&self, name: &str) -> StageView<'a> {
        self.stage_at(0, name)
    }

    /// One stage as it is configured in a repository.
    ///
    /// The scope with the longest path that contains `repo` wins, because a
    /// hook runs wherever the agent ran its tool and that is as often a
    /// subdirectory of the repository as its root. With no scope for it, the
    /// global view answers.
    pub fn stage_in(&self, repo: &Path, name: &str) -> StageView<'a> {
        self.stage_at(self.scope_for(repo), name)
    }

    /// The stages the snapshot holds, in the global view.
    pub fn stage_names(&self) -> Vec<&'a str> {
        let Some(scope) = self.scope(0) else {
            return Vec::new();
        };
        (0..scope.stage_count)
            .filter_map(|i| self.stage_rec(scope.stage_first + i))
            .map(|rec| self.text(u32_at(rec, 0), u32_at(rec, 4)))
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// The repository scopes the snapshot holds.
    pub fn repo_scopes(&self) -> Vec<&'a Path> {
        (1..self.scopes.len() / SCOPE_REC)
            .filter_map(|i| self.scope(i))
            .map(|scope| Path::new(scope.path))
            .collect()
    }

    /// Rebuild a configuration from what the snapshot holds.
    ///
    /// For `lessr config` printing a snapshot without re-reading the TOML. What
    /// comes back reproduces the *resolution*, not the layering that produced
    /// it: a snapshot stores the value and the layer that set it, not the
    /// layers it beat.
    pub fn to_config(&self) -> Config {
        let mut config = Config::new();
        for index in 0..self.scopes.len() / SCOPE_REC {
            let Some(scope) = self.scope(index) else {
                continue;
            };
            let repo = (!scope.path.is_empty()).then(|| Path::new(scope.path));
            for i in 0..scope.stage_count {
                let Some(rec) = self.stage_rec(scope.stage_first + i) else {
                    continue;
                };
                let view = self.stage_view(rec);
                let name = self.text(u32_at(rec, 0), u32_at(rec, 4));

                if let Some(reason) = view.floor_reason {
                    config.floor_mut().force_off(name, repo, reason);
                }
                let over = match (repo, name.is_empty()) {
                    (Some(path), true) => config.repo_default_mut(path),
                    (Some(path), false) => config.repo_stage_mut(path, name),
                    (None, true) => config.global_default_mut(),
                    (None, false) => config.global_stage_mut(name),
                };
                over.mode = Some(view.mode);
                over.level = Some(view.level);
                for setting in view.settings() {
                    over.settings.set(setting.key, setting.value);
                }
            }
        }
        config
    }

    /// The scope that fits this path best, or 0 (the global view).
    fn scope_for(&self, repo: &Path) -> usize {
        (1..self.scopes.len() / SCOPE_REC)
            .filter(|index| match self.scope(*index) {
                Some(scope) => !scope.path.is_empty() && repo.starts_with(scope.path),
                None => false,
            })
            .max_by_key(|index| self.scope(*index).map_or(0, |s| s.path.len()))
            .unwrap_or(0)
    }

    fn stage_at(&self, scope_index: usize, name: &str) -> StageView<'a> {
        let Some(scope) = self.scope(scope_index) else {
            return StageView::defaults();
        };
        let mut nameless = None;
        for i in 0..scope.stage_count {
            let Some(rec) = self.stage_rec(scope.stage_first + i) else {
                continue;
            };
            let found = self.text(u32_at(rec, 0), u32_at(rec, 4));
            if found == name {
                return self.stage_view(rec);
            }
            if found.is_empty() {
                nameless = Some(rec);
            }
        }
        // A stage the configuration never named still gets `[stages.default]`.
        nameless.map_or_else(StageView::defaults, |rec| self.stage_view(rec))
    }

    fn stage_view(&self, rec: &'a [u8]) -> StageView<'a> {
        let first = u32_at(rec, 8) as usize;
        let len = u32_at(rec, 12) as usize;
        let settings = first
            .checked_mul(SETTING_REC)
            .and_then(|start| Some(start..start.checked_add(len.checked_mul(SETTING_REC)?)?))
            .and_then(|range| self.settings.get(range))
            .unwrap_or(&[]);

        let reason = self.text(u32_at(rec, 16), u32_at(rec, 20));
        StageView {
            // An unknown tag is a value from a version we do not have, so it
            // reads as the default rather than as whatever it is nearest.
            mode: Mode::from_tag(rec[24]).unwrap_or_default(),
            level: Level::from_tag(rec[25]).unwrap_or_default(),
            mode_layer: Layer::from_tag(rec[26]).unwrap_or_default(),
            level_layer: Layer::from_tag(rec[27]).unwrap_or_default(),
            floor_reason: (!reason.is_empty()).then_some(reason),
            settings,
            blob: self.blob,
        }
    }

    fn scope(&self, index: usize) -> Option<ScopeRec<'a>> {
        let at = index.checked_mul(SCOPE_REC)?;
        let rec = self.scopes.get(at..at + SCOPE_REC)?;
        Some(ScopeRec {
            path: self.text(u32_at(rec, 0), u32_at(rec, 4)),
            stage_first: u32_at(rec, 8) as usize,
            stage_count: u32_at(rec, 12) as usize,
        })
    }

    fn stage_rec(&self, index: usize) -> Option<&'a [u8]> {
        let at = index.checked_mul(STAGE_REC)?;
        self.stages.get(at..at + STAGE_REC)
    }

    fn text(&self, off: u32, len: u32) -> &'a str {
        text(self.blob, off, len)
    }
}

/// One scope's header, decoded.
struct ScopeRec<'a> {
    path: &'a str,
    stage_first: usize,
    stage_count: usize,
}

/// A region of the file, checked to be inside it and a whole number of records.
fn region(bytes: &[u8], off: u32, count: u32, stride: usize) -> Result<&[u8]> {
    let off = off as usize;
    let len = (count as usize)
        .checked_mul(stride)
        .ok_or(Error::SnapshotCorrupt("a region is longer than the file"))?;
    if off < HEADER_LEN && len > 0 {
        return Err(Error::SnapshotCorrupt("a region overlaps the header"));
    }
    off.checked_add(len)
        .and_then(|end| bytes.get(off..end))
        .ok_or(Error::SnapshotCorrupt(
            "a region runs past the end of the file",
        ))
}

fn text(blob: &[u8], off: u32, len: u32) -> &str {
    let at = off as usize;
    at.checked_add(len as usize)
        .and_then(|end| blob.get(at..end))
        .and_then(|b| std::str::from_utf8(b).ok())
        .unwrap_or("")
}

/// What one stage runs with, as the snapshot holds it.
///
/// Answering `mode`, `level` or a tunable from one of these costs a bounds
/// check and a byte comparison: no parsing, no allocation, nothing mapped in
/// that was not already mapped.
#[derive(Clone, Copy, Debug)]
pub struct StageView<'a> {
    mode: Mode,
    level: Level,
    mode_layer: Layer,
    level_layer: Layer,
    floor_reason: Option<&'a str>,
    settings: &'a [u8],
    blob: &'a [u8],
}

impl<'a> StageView<'a> {
    /// The compiled-in defaults, for a stage the snapshot says nothing about.
    const fn defaults() -> Self {
        Self {
            mode: Mode::Shadow,
            level: Level::Balanced,
            mode_layer: Layer::Default,
            level_layer: Layer::Default,
            floor_reason: None,
            settings: &[],
            blob: &[],
        }
    }

    /// Whether the stage runs, and whether its output is applied.
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// How much it cuts. Never whether the safety checks run — see
    /// [`crate::Level`].
    pub const fn level(&self) -> Level {
        self.level
    }

    /// Which layer set the mode. This is what `lessr config --explain` prints,
    /// and it is why a snapshot can answer "why is my setting not taking
    /// effect" without the TOML.
    pub const fn mode_layer(&self) -> Layer {
        self.mode_layer
    }

    /// Which layer set the level.
    pub const fn level_layer(&self) -> Layer {
        self.level_layer
    }

    /// Why the self-healing table turned this stage off, if it did
    /// (loop-safety 5, which requires the reason be reported).
    pub const fn floor_reason(&self) -> Option<&'a str> {
        self.floor_reason
    }

    /// Whether the self-healing table is what is holding this stage off.
    pub const fn forced_off(&self) -> bool {
        matches!(self.mode_layer, Layer::SafetyFloor)
    }

    /// Apply the environment overrides on top of the snapshot.
    ///
    /// The snapshot is written once, by the CLI; `LESSR_STAGE_GATE_MODE=off` is
    /// set by whoever runs the agent, so the last layer is applied here, at the
    /// point of use. The safety floor still beats it: a stage the self-healing
    /// table turned off stays off, and no environment variable raises it.
    pub fn with_env(mut self, stage: &str, env: &EnvOverrides) -> Self {
        if self.forced_off() {
            return self;
        }
        if let Some(over) = env.stage(stage) {
            if let Some(mode) = over.mode {
                self.mode = mode;
                self.mode_layer = Layer::Env;
            }
            if let Some(level) = over.level {
                self.level = level;
                self.level_layer = Layer::Env;
            }
        }
        self
    }

    /// A whole number, or `default` if the key is absent or the value is not
    /// one.
    pub fn u64(&self, key: &str, default: u64) -> u64 {
        self.scalar(key).and_then(Scalar::as_u64).unwrap_or(default)
    }

    /// A flag, or `default` if the key is absent or the value is not one.
    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.scalar(key)
            .and_then(Scalar::as_bool)
            .unwrap_or(default)
    }

    /// Text, or `default` if the key is absent or the value is not text.
    ///
    /// The borrow is into the snapshot itself, so this copies nothing. The
    /// default may be shorter-lived than the mapping; what comes back lives as
    /// long as whichever of the two the caller can hold.
    pub fn str<'s>(&self, key: &str, default: &'s str) -> &'s str
    where
        'a: 's,
    {
        self.scalar(key).and_then(Scalar::as_str).unwrap_or(default)
    }

    /// Which layer set one tunable, or `None` if nothing did.
    pub fn setting_layer(&self, key: &str) -> Option<Layer> {
        let rec = self.record(key)?;
        Layer::from_tag(rec[9])
    }

    /// Every tunable, in the order the layers set them. Allocates a [`Value`]
    /// per setting, so it is for `lessr config`, not for the hook path.
    pub fn settings(&self) -> impl Iterator<Item = SettingView<'a>> + use<'a> {
        let blob = self.blob;
        self.settings.chunks_exact(SETTING_REC).map(move |rec| {
            SettingView {
                key: text(blob, u32_at(rec, 0), u32_at(rec, 4)),
                value: scalar(rec, blob).map_or(Value::Int(0), Scalar::to_value),
                // An unknown tag means a version we do not have; the value is
                // still the value, we just cannot say who set it.
                layer: Layer::from_tag(rec[9]).unwrap_or_default(),
            }
        })
    }

    /// How many tunables this stage carries.
    pub fn settings_len(&self) -> usize {
        self.settings.len() / SETTING_REC
    }

    fn record(&self, key: &str) -> Option<&'a [u8]> {
        self.settings
            .chunks_exact(SETTING_REC)
            .find(|rec| text(self.blob, u32_at(rec, 0), u32_at(rec, 4)) == key)
    }

    fn scalar(&self, key: &str) -> Option<Scalar<'a>> {
        scalar(self.record(key)?, self.blob)
    }
}

/// One setting record, decoded.
fn scalar<'a>(rec: &[u8], blob: &'a [u8]) -> Option<Scalar<'a>> {
    let payload = u64_at(rec, 12);
    match ValueKind::from_tag(rec[8])? {
        ValueKind::Bool => Some(Scalar::Bool(payload != 0)),
        ValueKind::Int => Some(Scalar::Int(payload as i64)),
        ValueKind::Str => Some(Scalar::Str(text(
            blob,
            (payload >> 32) as u32,
            (payload & 0xffff_ffff) as u32,
        ))),
    }
}

/// One tunable as the snapshot holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingView<'a> {
    /// The key, as the config file spelled it.
    pub key: &'a str,
    /// The value in force.
    pub value: Value,
    /// The layer that set it.
    pub layer: Layer,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StageOverride;

    fn repo() -> &'static Path {
        Path::new("/home/dev/work/monorepo")
    }

    /// The configuration from the example in `docs/CONFIG.md`.
    fn config() -> Config {
        let mut config = Config::new();
        *config.global_default_mut() = StageOverride::new().with_mode(Mode::Shadow);
        *config.global_stage_mut("gate") = StageOverride::new()
            .with_mode(Mode::Active)
            .with_level(Level::Balanced)
            .with("max_repeated_lines", 3u64)
            .with("strip_timestamps", true)
            .with("summary_style", "tail");
        *config.global_stage_mut("trap") = StageOverride::new()
            .with_mode(Mode::Active)
            .with_level(Level::Safe)
            .with("json_bytes", 131072u64);
        *config.repo_stage_mut(repo(), "gate") = StageOverride::new().with_mode(Mode::Off);
        config
    }

    #[test]
    fn a_snapshot_round_trips_every_stage_in_every_scope() {
        let config = config();
        let bytes = Snapshot::encode(&config);
        let view = SnapshotView::parse(&bytes).unwrap();

        for name in config.stage_names() {
            let resolved = config.resolve_upto(name, None, Layer::RepoStage);
            let stage = view.stage(name);
            assert_eq!(stage.mode(), resolved.mode(), "{name} mode");
            assert_eq!(stage.level(), resolved.level(), "{name} level");
            assert_eq!(stage.mode_layer(), resolved.mode_layer(), "{name} whence");
            assert_eq!(stage.settings_len(), resolved.settings().len());

            for (key, value) in resolved.settings().iter() {
                assert_eq!(
                    stage.u64(key, 0),
                    value.as_u64().unwrap_or(0),
                    "{name}.{key}"
                );
                assert_eq!(stage.setting_layer(key), resolved.setting_layer(key));
            }
        }

        let gate = view.stage("gate");
        assert_eq!(gate.u64("max_repeated_lines", 0), 3);
        assert!(gate.bool("strip_timestamps", false));
        assert_eq!(gate.str("summary_style", "head"), "tail");
        assert_eq!(view.stage("trap").u64("json_bytes", 0), 131072);
    }

    #[test]
    fn the_hook_reads_the_repo_scope_it_is_running_in() {
        let bytes = Snapshot::encode(&config());
        let view = SnapshotView::parse(&bytes).unwrap();

        assert_eq!(view.stage("gate").mode(), Mode::Active, "globally");
        assert_eq!(view.stage_in(repo(), "gate").mode(), Mode::Off);
        assert_eq!(
            view.stage_in(&repo().join("crates/lessr-gate"), "gate")
                .mode(),
            Mode::Off,
            "and in a subdirectory of it"
        );
        assert_eq!(
            view.stage_in(Path::new("/home/dev/elsewhere"), "gate")
                .mode(),
            Mode::Active,
            "but nowhere else"
        );
        assert_eq!(
            view.stage_in(repo(), "trap").mode(),
            Mode::Active,
            "the repo scope carries the stages it does not override"
        );
    }

    #[test]
    fn a_stage_the_snapshot_never_heard_of_reads_as_the_defaults() {
        let bytes = Snapshot::encode(&config());
        let view = SnapshotView::parse(&bytes).unwrap();

        let unknown = view.stage("compact");
        assert_eq!(unknown.mode(), Mode::Shadow);
        assert_eq!(unknown.level(), Level::Balanced);
        assert_eq!(unknown.level_layer(), Layer::Default);
        assert_eq!(unknown.u64("anything", 7), 7);
        assert_eq!(unknown.settings_len(), 0);
    }

    #[test]
    fn a_stage_nobody_named_still_gets_the_global_default() {
        // A configuration that says nothing but `[stages.default]`. Every
        // stage the binary registers has to see it, and none of them are named
        // anywhere for the snapshot to hang a record on.
        let mut config = Config::new();
        *config.global_default_mut() = StageOverride::new()
            .with_mode(Mode::Active)
            .with_level(Level::Safe)
            .with("min_bytes", 2048u64);
        *config.repo_default_mut(repo()) = StageOverride::new().with_mode(Mode::Off);

        let bytes = Snapshot::encode(&config);
        let view = SnapshotView::parse(&bytes).unwrap();

        let gate = view.stage("gate");
        assert_eq!(gate.mode(), Mode::Active);
        assert_eq!(gate.mode_layer(), Layer::GlobalDefault);
        assert_eq!(gate.level(), Level::Safe);
        assert_eq!(gate.u64("min_bytes", 0), 2048);
        assert!(view.stage_names().is_empty(), "and it is still not a stage");

        let in_repo = view.stage_in(repo(), "gate");
        assert_eq!(in_repo.mode(), Mode::Off);
        assert_eq!(in_repo.mode_layer(), Layer::RepoDefault);
        assert_eq!(in_repo.level(), Level::Safe, "from the layer below");
    }

    #[test]
    fn a_bad_magic_falls_back_to_the_defaults() {
        let mut bytes = Snapshot::encode(&config());
        bytes[0] = b'X';

        assert!(matches!(
            SnapshotView::parse(&bytes),
            Err(Error::SnapshotMagic)
        ));
        let view = SnapshotView::read_or_defaults(&bytes);
        assert_eq!(view.stage("gate").mode(), Mode::Shadow, "not Active");
        assert_eq!(view.stage("gate").u64("max_repeated_lines", 99), 99);
    }

    #[test]
    fn a_version_this_build_does_not_know_falls_back_to_the_defaults() {
        let mut bytes = Snapshot::encode(&config());
        bytes[8..10].copy_from_slice(&(VERSION + 1).to_le_bytes());

        match SnapshotView::parse(&bytes) {
            Err(Error::SnapshotVersion { found, expected }) => {
                assert_eq!(found, VERSION + 1);
                assert_eq!(expected, VERSION);
            }
            other => panic!("expected a version error, got {other:?}"),
        }
        assert_eq!(
            SnapshotView::read_or_defaults(&bytes).stage("gate").mode(),
            Mode::Shadow,
            "old bytes are never read as new ones"
        );
    }

    #[test]
    fn a_truncated_or_edited_snapshot_falls_back_to_the_defaults() {
        let whole = Snapshot::encode(&config());

        let truncated = &whole[..whole.len() - 8];
        assert!(SnapshotView::parse(truncated).is_err());
        assert_eq!(
            SnapshotView::read_or_defaults(truncated)
                .stage("gate")
                .mode(),
            Mode::Shadow
        );

        let mut edited = whole.clone();
        let last = edited.len() - 1;
        edited[last] ^= 0xff;
        assert!(matches!(
            SnapshotView::parse(&edited),
            Err(Error::SnapshotCorrupt(_))
        ));

        assert!(SnapshotView::parse(&[]).is_err(), "an empty file");
        assert_eq!(
            SnapshotView::read_or_defaults(&[]).stage("gate").mode(),
            Mode::Shadow
        );
    }

    #[test]
    fn an_offset_that_runs_off_the_end_is_refused_not_followed() {
        let mut bytes = Snapshot::encode(&config());
        // Point the stage table past the end of the file.
        bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes());
        let digest = digest(&bytes[HEADER_LEN..]);
        bytes[48..56].copy_from_slice(&digest.to_le_bytes());

        assert!(matches!(
            SnapshotView::parse(&bytes),
            Err(Error::SnapshotCorrupt(_))
        ));
    }

    #[test]
    fn the_safety_floor_is_written_into_the_snapshot_with_its_reason() {
        let mut config = config();
        config
            .floor_mut()
            .force_off("gate", Some(repo()), "handles expanded on 14 % of outputs");
        let bytes = Snapshot::encode(&config);
        let view = SnapshotView::parse(&bytes).unwrap();

        let gate = view.stage_in(repo(), "gate");
        assert_eq!(gate.mode(), Mode::Off);
        assert_eq!(gate.mode_layer(), Layer::SafetyFloor);
        assert!(gate.forced_off());
        assert_eq!(
            gate.floor_reason(),
            Some("handles expanded on 14 % of outputs")
        );
        assert!(!view.stage("gate").forced_off(), "only in that repo");
    }

    #[test]
    fn the_environment_is_not_baked_into_the_snapshot() {
        let mut config = config();
        config.set_env(EnvOverrides::from_pairs([("LESSR_STAGE_GATE_MODE", "off")]));

        let bytes = Snapshot::encode(&config);
        let view = SnapshotView::parse(&bytes).unwrap();
        assert_eq!(
            view.stage("gate").mode(),
            Mode::Active,
            "one shell's environment does not belong in a file everyone reads"
        );
    }

    #[test]
    fn the_environment_applies_at_the_point_of_use() {
        let bytes = Snapshot::encode(&config());
        let view = SnapshotView::parse(&bytes).unwrap();
        let env = EnvOverrides::from_pairs([
            ("LESSR_STAGE_GATE_MODE", "off"),
            ("LESSR_STAGE_TRAP_LEVEL", "aggressive"),
        ]);

        let gate = view.stage("gate").with_env("gate", &env);
        assert_eq!(gate.mode(), Mode::Off);
        assert_eq!(gate.mode_layer(), Layer::Env);

        let trap = view.stage("trap").with_env("trap", &env);
        assert_eq!(trap.level(), Level::Aggressive);
        assert_eq!(trap.mode(), Mode::Active, "the snapshot still holds");

        let unknown = view.stage("compact").with_env("compact", &env);
        assert_eq!(unknown.mode(), Mode::Shadow, "nothing said about it");
    }

    #[test]
    fn the_environment_cannot_raise_a_stage_the_floor_turned_off() {
        let mut config = config();
        config.floor_mut().force_off("trap", None, "expanded");
        let bytes = Snapshot::encode(&config);
        let view = SnapshotView::parse(&bytes).unwrap();

        let env = EnvOverrides::from_pairs([("LESSR_STAGE_TRAP_MODE", "active")]);
        let trap = view.stage("trap").with_env("trap", &env);
        assert_eq!(trap.mode(), Mode::Off);
        assert_eq!(trap.mode_layer(), Layer::SafetyFloor);
    }

    #[test]
    fn a_string_default_need_not_outlive_the_snapshot() {
        let bytes = Snapshot::encode(&config());
        let view = SnapshotView::parse(&bytes).unwrap();
        let fallback = String::from("head");
        assert_eq!(view.stage("gate").str("summary_style", &fallback), "tail");
        assert_eq!(view.stage("gate").str("nothing", &fallback), "head");
    }

    #[test]
    fn a_string_setting_is_borrowed_from_the_snapshot_not_copied() {
        let bytes = Snapshot::encode(&config());
        let view = SnapshotView::parse(&bytes).unwrap();

        let value = view.stage("gate").str("summary_style", "head");
        let base = bytes.as_ptr() as usize;
        let at = value.as_ptr() as usize;
        assert!(
            (base..base + bytes.len()).contains(&at),
            "the borrow points into the snapshot itself"
        );
    }

    #[test]
    fn strings_are_stored_once_however_often_they_repeat() {
        let mut config = Config::new();
        for stage in ["gate", "trap", "dedup", "detect"] {
            *config.global_stage_mut(stage) = StageOverride::new().with("min_bytes", 2048u64);
        }
        let bytes = Snapshot::encode(&config);
        let blob = &bytes[u32_at(&bytes, 40) as usize..];
        assert_eq!(
            blob.windows(9).filter(|w| *w == b"min_bytes").count(),
            1,
            "the key is shared by every record that names it"
        );
    }

    #[test]
    fn to_config_reproduces_the_resolution() {
        let mut original = config();
        original
            .floor_mut()
            .force_off("trap", Some(repo()), "expanded");
        let bytes = Snapshot::encode(&original);
        let rebuilt = SnapshotView::parse(&bytes).unwrap().to_config();

        for scope in [None, Some(repo())] {
            for name in original.stage_names() {
                let was = original.resolve_upto(name, scope, Layer::RepoStage);
                let now = rebuilt.resolve_upto(name, scope, Layer::RepoStage);
                assert_eq!(now.mode(), was.mode(), "{name} in {scope:?}");
                assert_eq!(now.level(), was.level(), "{name} in {scope:?}");
                assert_eq!(now.settings(), was.settings(), "{name} in {scope:?}");
                assert_eq!(now.floor_reason(), was.floor_reason());
            }
        }
    }

    #[test]
    fn an_empty_config_is_a_valid_snapshot_of_nothing() {
        let bytes = Snapshot::encode(&Config::new());
        let view = SnapshotView::parse(&bytes).unwrap();
        assert!(view.stage_names().is_empty());
        assert!(view.repo_scopes().is_empty());
        assert_eq!(view.stage("gate").mode(), Mode::Shadow);
    }

    #[test]
    fn a_snapshot_survives_the_process_that_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state").join("config.bin");
        Snapshot::write(&path, &config()).unwrap();

        let bytes = Snapshot::read(&path).unwrap();
        let view = SnapshotView::parse(&bytes).unwrap();
        assert_eq!(view.stage("gate").mode(), Mode::Active);
        assert_eq!(view.stage_in(repo(), "gate").mode(), Mode::Off);

        // Rewriting replaces the file whole, never truncates it in place.
        let mut second = config();
        *second.global_stage_mut("gate") = StageOverride::new().with_mode(Mode::Shadow);
        Snapshot::write(&path, &second).unwrap();
        let bytes = Snapshot::read(&path).unwrap();
        assert_eq!(
            SnapshotView::parse(&bytes).unwrap().stage("gate").mode(),
            Mode::Shadow
        );
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1,
            "no temporary file left behind"
        );
    }

    #[test]
    fn the_format_is_the_one_this_module_documents() {
        // These are on disk. Changing any of them without bumping the version
        // is what makes an old snapshot mean something new.
        assert_eq!(Snapshot::MAGIC, *b"LESSRCFG");
        assert_eq!(Snapshot::VERSION, 1);
        assert_eq!(
            (HEADER_LEN, SCOPE_REC, STAGE_REC, SETTING_REC),
            (56, 16, 28, 20)
        );

        let mut config = Config::new();
        *config.global_stage_mut("gate") = StageOverride::new()
            .with_mode(Mode::Active)
            .with("max_repeated_lines", 3u64);
        let bytes = Snapshot::encode(&config);

        assert_eq!(&bytes[..8], b"LESSRCFG");
        assert_eq!(u16_at(&bytes, 8), Snapshot::VERSION);
        assert_eq!(u16_at(&bytes, 10), 0, "flags are reserved");
        assert_eq!(u32_at(&bytes, 12) as usize, bytes.len());
        assert_eq!(u32_at(&bytes, 16) as usize, HEADER_LEN, "scopes come first");
        assert_eq!(u32_at(&bytes, 20), 1, "the global scope");
        assert_eq!(u32_at(&bytes, 28), 2, "the nameless default, and one stage");
        assert_eq!(u32_at(&bytes, 36), 1, "one setting");
        assert_eq!(u64_at(&bytes, 48), digest(&bytes[HEADER_LEN..]));
    }

    #[test]
    fn the_settings_of_a_stage_come_back_with_their_layers() {
        let bytes = Snapshot::encode(&config());
        let view = SnapshotView::parse(&bytes).unwrap();
        let settings: Vec<_> = view.stage("gate").settings().collect();

        assert_eq!(settings.len(), 3);
        assert_eq!(settings[0].key, "max_repeated_lines");
        assert_eq!(settings[0].value, Value::Int(3));
        assert_eq!(settings[0].layer, Layer::GlobalStage);
        assert_eq!(settings[2].value, Value::Str("tail".into()));
    }
}
