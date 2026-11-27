use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use mw_core::tileset::TilesetAtlas;
use mw_level::{LevelBundle, ValidationError, LEVEL_FILE};
use serde_json::{json, Value};

/// Filesystem operations, independent of MCP. One server serializes access to this store.
pub struct LevelStore {
    root: PathBuf,
    atlas: TilesetAtlas,
    /// Auto-tiled terrain families of the atlas.
    terrains: BTreeSet<String>,
    /// Every id a legend may use: terrain families and named pieces. Built once from the
    /// atlas; `palette()` and every validation read it, so they cannot disagree.
    tile_ids: BTreeSet<String>,
}

impl LevelStore {
    pub fn new(levels: impl AsRef<Path>, tileset: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(&levels).context("creating levels directory")?;
        let atlas = TilesetAtlas::load(tileset).context("loading tileset atlas")?;
        let terrains: BTreeSet<String> = atlas
            .pieces
            .values()
            .filter_map(|p| p.autotile.as_ref().map(|a| a.terrain.clone()))
            .collect();
        let tile_ids = terrains
            .iter()
            .cloned()
            .chain(atlas.pieces.keys().cloned())
            .collect();
        Ok(Self {
            root: levels.as_ref().canonicalize()?,
            atlas,
            terrains,
            tile_ids,
        })
    }

    pub fn palette(&self) -> Value {
        json!({"entity_kinds": mw_level::KNOWN_ENTITY_KINDS,
            "terrains": self.terrains, "pieces": self.atlas.pieces})
    }

    /// Ids accepted in a legend (terrain families and atlas pieces).
    pub fn tile_ids(&self) -> &BTreeSet<String> {
        &self.tile_ids
    }

    /// Every validation error of `bundle`, legend ids checked against the atlas
    /// (`unknown_tile_id`). The single validation used by reads, writes and `validate_level`.
    pub fn validate(&self, bundle: &LevelBundle) -> Vec<ValidationError> {
        bundle.validate_with_tile_ids(&self.tile_ids)
    }

    pub fn list(&self) -> Result<Value> {
        let mut levels = Vec::new();
        let mut entries = fs::read_dir(&self.root)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let id = entry.file_name().to_string_lossy().into_owned();
            if valid_id(&id)
                && entry.file_type()?.is_dir()
                && entry.path().join(LEVEL_FILE).exists()
            {
                // A malformed level must not hide the other levels from an editor.
                match self.load(&id) {
                    Ok(bundle) => levels.push(json!({"id": id, "name": bundle.level.name,
                        "size": bundle.level.size, "errors": self.validate(&bundle)})),
                    Err(error) => levels.push(json!({"id": id, "error": format!("{error:#}")})),
                }
            }
        }
        Ok(json!({"levels": levels}))
    }

    pub fn load(&self, id: &str) -> Result<LevelBundle> {
        let dir = self.directory(id)?;
        reject_symlinks(&dir)?;
        let bundle = LevelBundle::load(&dir).with_context(|| format!("loading level `{id}`"))?;
        if bundle.level.id != id {
            bail!(
                "level id `{}` differs from directory `{id}`",
                bundle.level.id
            );
        }
        Ok(bundle)
    }

    pub fn get(&self, id: &str) -> Result<Value> {
        let bundle = self.load(id)?;
        let grids: BTreeMap<_, _> = bundle
            .level
            .layers
            .iter()
            .map(|l| (&l.name, l.tiles.join("\n")))
            .collect();
        let entities: Vec<_> = bundle
            .level
            .entities
            .iter()
            .enumerate()
            .map(|(index, entity)| json!({"index": index, "entity": entity}))
            .collect();
        Ok(json!({"level": bundle.level, "grids_top_to_bottom": grids,
            "indexed_entities": entities, "errors": self.validate(&bundle)}))
    }

    /// Validate first, then publish a newly created directory with one rename.
    pub fn create(&self, bundle: &LevelBundle) -> Result<()> {
        self.check(bundle)?;
        let dest = self.directory(&bundle.level.id)?;
        if dest.try_exists()? {
            bail!(
                "level `{}` already exists; create_level never overwrites",
                bundle.level.id
            );
        }
        let staging = tempfile::tempdir_in(&self.root)?;
        bundle.save(staging.path())?;
        fs::rename(staging.path(), &dest).context("publishing new level")?;
        Ok(())
    }

    /// Each file is replaced atomically. Scripts are published before their level references.
    /// All validation and staging complete before any existing file is replaced.
    pub fn save(&self, bundle: &LevelBundle) -> Result<()> {
        self.check(bundle)?;
        let dir = self.directory(&bundle.level.id)?;
        reject_symlinks(&dir)?;
        let mut staged = Vec::new();
        for (rel, source) in &bundle.scripts {
            staged.push(stage_file(&dir.join(rel), source.as_bytes())?);
        }
        staged.push(stage_file(
            &dir.join(LEVEL_FILE),
            bundle.level.to_json()?.as_bytes(),
        )?);
        for (file, path) in staged {
            file.persist(&path)
                .with_context(|| format!("replacing {}", path.display()))?;
        }
        Ok(())
    }

    fn check(&self, bundle: &LevelBundle) -> Result<()> {
        let errors = self.validate(bundle);
        if !errors.is_empty() {
            bail!("validation failed: {}", serde_json::to_string(&errors)?);
        }
        // save() also sees script sources supplied by callers outside the MCP handlers.
        if bundle
            .scripts
            .keys()
            .any(|p| !mw_level::is_valid_script_path(p))
        {
            bail!("script paths must be relative scripts/**/*.rhai paths without traversal");
        }
        Ok(())
    }

    fn directory(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            bail!("invalid level id: use 1..=64 characters from [a-z0-9_]");
        }
        Ok(self.root.join(id))
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn reject_symlinks(path: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("symlinks are not allowed inside levels: {}", path.display());
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            reject_symlinks(&entry?.path())?;
        }
    }
    Ok(())
}

fn stage_file(path: &Path, bytes: &[u8]) -> Result<(tempfile::NamedTempFile, PathBuf)> {
    let parent = path.parent().context("file needs a parent directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    Ok((file, path.to_owned()))
}
