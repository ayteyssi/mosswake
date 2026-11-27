//! [`LevelSync`]: the client side of level transfer. Feed it every server message; it
//! answers offers (from the cache or by downloading), reassembles and verifies the package,
//! loads the level and tells the server when it is ready to play.
//!
//! Scripts arrive with the level but are never run on clients (they run on the server).

use anyhow::{bail, Context, Result};
use mw_level::LevelBundle;
use mw_net::{Assembler, ClientMessage, LevelCache, LevelOffer, ServerMessage};

/// Client-side level state.
#[derive(Debug, Default)]
pub struct LevelSync {
    cache: LevelCache,
    download: Option<Assembler>,
    /// Transfer number of the loaded level, and the level.
    loaded: Option<(u32, LevelBundle)>,
    /// Packages downloaded (not served from the cache), for tests and logs.
    downloads: u32,
}

/// What handling a message produced.
#[derive(Debug, Default)]
pub struct SyncOutcome {
    /// Messages to send to the server.
    pub replies: Vec<ClientMessage>,
    /// A new level was just loaded (the game must reset its world to it).
    pub loaded: Option<LevelBundle>,
}

impl LevelSync {
    pub fn new(cache: LevelCache) -> Self {
        Self {
            cache,
            ..Self::default()
        }
    }

    /// Handles `message` if it is about levels (other messages are ignored). An error means
    /// the server sent an unusable level: the caller should disconnect.
    pub fn handle(&mut self, message: &ServerMessage) -> Result<SyncOutcome> {
        match message {
            ServerMessage::LevelOffer(offer) => self.on_offer(*offer),
            ServerMessage::LevelChunk {
                transfer,
                index,
                data,
            } => self.on_chunk(*transfer, *index, data.clone()),
            _ => Ok(SyncOutcome::default()),
        }
    }

    fn on_offer(&mut self, offer: LevelOffer) -> Result<SyncOutcome> {
        // A new offer replaces whatever was loaded or downloading.
        self.loaded = None;
        self.download = None;
        if let Some(package) = self.cache.get(&offer.hash) {
            return self.finish(offer.transfer, &package);
        }
        self.download = Some(Assembler::new(offer).context("refusing the offered level")?);
        Ok(SyncOutcome {
            replies: vec![ClientMessage::LevelRequest {
                transfer: offer.transfer,
            }],
            loaded: None,
        })
    }

    fn on_chunk(&mut self, transfer: u32, index: u32, data: Vec<u8>) -> Result<SyncOutcome> {
        let Some(download) = self.download.as_mut() else {
            return Ok(SyncOutcome::default());
        };
        if download.offer().transfer != transfer {
            return Ok(SyncOutcome::default()); // chunk of an older offer
        }
        let Some(package) = download.add(index, data).context("level transfer failed")? else {
            return Ok(SyncOutcome::default());
        };
        self.download = None;
        self.downloads += 1;
        if let Err(e) = self.cache.put(&package) {
            eprintln!("warning: cannot cache level: {e}");
        }
        self.finish(transfer, &package)
    }

    fn finish(&mut self, transfer: u32, package: &[u8]) -> Result<SyncOutcome> {
        let bundle = LevelBundle::from_package(package).context("invalid level package")?;
        let errors = bundle.validate();
        if !errors.is_empty() {
            bail!("server sent an invalid level: {errors:?}");
        }
        self.loaded = Some((transfer, bundle.clone()));
        Ok(SyncOutcome {
            replies: vec![ClientMessage::LevelReady { transfer }],
            loaded: Some(bundle),
        })
    }

    /// The level being played, once loaded. Snapshots before that refer to a world the
    /// client does not have and should be ignored.
    pub fn level(&self) -> Option<&LevelBundle> {
        self.loaded.as_ref().map(|(_, bundle)| bundle)
    }

    /// Download progress of the pending level (`None` when nothing is downloading).
    pub fn progress(&self) -> Option<f32> {
        self.download.as_ref().map(Assembler::progress)
    }

    /// Packages actually downloaded (cache hits not counted).
    pub fn downloads(&self) -> u32 {
        self.downloads
    }
}
