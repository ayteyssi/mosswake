# Assets

Toutes les animations sont des **séquences de frames PNG** numérotées `0000.png`, `0001.png`, …

| Dossier | Frames | Taille frame |
|---|---|---|
| `characters/blue_wizard/idle` | 20 | 512×512 |
| `characters/blue_wizard/walk` | 20 | 512×512 |
| `characters/blue_wizard/jump` | 8 | 512×512 |
| `characters/blue_wizard/dash2`, `dash3`, `dash_effect` | 16 | 512×512 |
| `enemies/slime_green` | 30 | 376×256 |
| `enemies/slime_orange` | 30 | 510×410 |
| `plants/blue_flower_1`, `blue_flower_2` | 60 | 768×768 |
| `plants/plant_1..3` | 90 | 512×512 |
| `plants/plant_4..7` | 60 | 512×512 |
| `plants/plant_8_poison`, `plant_wind_1` | 30 | 512×512 |
| `plants/plant_jump`, `plant_jump2` | 20 | 512×512 |

Tilesets (`tilesets/mossy/`, grandes planches peintes) :
| Fichier | Taille | Contenu (voir « Atlas du tileset ») |
|---|---|---|
| `tile_set.png` | 3584×3584 | grille 7×7 de 512 px : terrain auto-tilé (47 cas) + 2 décors « trou » |
| `floating_platforms.png` | 2048×2048 | 8 plateformes (1×1 à 4×1, une 1×2) |
| `decorations_and_hazards.png` | 4096×4096 | 3 rochers, 6 lianes à épines (hazards), 13 petites plantes, 4 troncs |
| `background_decoration.png` | 4096×4096 | 6 grandes silhouettes d'arrière-plan |
| `hanging_plants.png` | 2048×2948 | 10 plantes pendantes (fougères, mousses, feuilles) |
| `mossy_hills.png` | 2048×2048 | 7 buttes et touffes de mousse |

Les sprites sont haute résolution : ils seront affichés à l'échelle (ex. wizard ≈ 2 tiles de haut).

## Atlas du tileset (`assets/tilesets/mossy/atlas.json`)
Types, chargement et validation : `mw_core::tileset` (`TilesetAtlas::load`, `validate`,
`autotile_candidates`).

### Stratégie
- **Échelle** : l'art du tileset est à **512 px par tile** (`pixels_per_tile`). Une tile de niveau
  vaut toujours `tile_size` = 32 unités monde : le rendu met à l'échelle (`32 / 512`).
- **Terrain (`tile_set.png`) = autotiling « blob » à 47 cas.** Chaque cellule de 512 px est dessinée
  pour une configuration de voisins (bord, coin extérieur, coin intérieur, bande d'une tile, bloc
  isolé…). Le niveau ne contient que `#` (terrain) ; le rendu calcule, pour chaque cellule pleine,
  son voisinage 3×3 et choisit la pièce dont le motif `neighbors` correspond. Les 47 configurations
  sont couvertes, **sans ambiguïté** (test exhaustif sur les 256 voisinages). Les deux cellules
  « anneau de mousse autour d'un trou » ne sont pas des cas d'autotiling mais des décors
  (`mossy_hole_a/b`) à poser sur une masse pleine.
- **Plateformes, décors, hazards, parallax** = sprites libres : un rectangle source, une emprise en
  tiles (`size`) et un ancrage. Le dessin peut déborder de son emprise (la mousse dépasse).
- **Parallax** (`background_decoration`, `mossy_hills`) : silhouettes à dessiner sur des couches
  d'arrière-plan défilant plus lentement ; elles peuvent aussi servir de décor de premier plan.

### Format
```json
{
  "version": 1,
  "pixels_per_tile": 512,
  "sheets": { "tile_set": { "file": "tile_set.png", "size": { "w": 3584, "h": 3584 } } },
  "pieces": {
    "mossy_ground_0_0": {
      "sheet": "tile_set", "rect": { "x": 0, "y": 0, "w": 512, "h": 512 },
      "kind": "terrain", "size": { "w": 1, "h": 1 }, "collision": true, "anchor": "center",
      "autotile": { "terrain": "mossy_ground", "neighbors": "*.*/.##/*##" }
    },
    "platform_2x1_a": {
      "sheet": "floating_platforms", "rect": { "x": 479, "y": 36, "w": 1049, "h": 450 },
      "kind": "platform", "size": { "w": 2, "h": 1 }, "collision": true, "anchor": "center"
    }
  }
}
```
| Champ de pièce | Description |
|---|---|
| `sheet`, `rect` | Planche et rectangle source (pixels, origine en haut à gauche, y vers le bas). Taille dessinée en tiles = `rect / pixels_per_tile`. |
| `kind` | `terrain`, `platform` (on tient dessus), `decor`, `hazard` (blesse), `parallax`. |
| `size` | Emprise sur la grille, en tiles entières. |
| `collision` | Emprise solide ou non (vrai pour `terrain` et `platform`). |
| `anchor` | Alignement du dessin sur l'emprise, toujours centré horizontalement : `center`, `bottom` (posé au sol), `top` (pendu au plafond). |
| `autotile` | Pour `terrain` uniquement : famille (`terrain`, l'id qu'utilise une légende de niveau) et motif `neighbors`. |

**Motif `neighbors`** : trois lignes de trois caractères séparées par `/`, du haut vers le bas,
la cellule elle-même au centre. `#` = même terrain, `.` = autre chose, `*` = indifférent.
Un coin n'est significatif que si ses deux côtés adjacents sont pleins (sinon il vaut `*`) :
c'est ce qui réduit 256 voisinages à 47 cas. Exemple : `"*.*/.##/*##"` = coin haut-gauche d'un bloc.
Si plusieurs pièces ont le même motif, ce sont des variantes : en choisir une de façon
déterministe (ex. selon la position).

### Comment l'atlas a été produit
Les pièces libres ont été détectées automatiquement (composantes connexes du canal alpha,
fragments isolés rattachés à la pièce la plus proche), puis corrigées et nommées à la main.
Les motifs du terrain ont été déduits en échantillonnant le bord de chaque cellule (corps noir =
voisin plein), puis vérifiés en rendant une carte de test auto-tilée. Aucun rectangle ne contient
de pixel opaque d'une autre pièce (les rectangles de `hanging_leaf_a` et `hanging_moss_b` se
touchent sur 4 px transparents). Le fichier est désormais la **source de vérité** : on l'édite à la main, et les tests
(`crates/mw_core/tests/tileset.rs`) vérifient bornes, tailles des planches et couverture de l'autotiling.

## Manifest des animations (`assets/manifest.json`)
Décrit chaque animation. Types et chargement : `mw_core::assets` (`AnimationManifest::load`,
`validate`). Un test vérifie que le manifest réel est valide **et** que chaque dossier de frames
de `assets/` y est référencé : **tout nouvel asset animé doit être ajouté au manifest**.

```json
{
  "version": 1,
  "sprites": {
    "slime_green": {
      "frame_size": { "w": 376, "h": 256 },
      "pivot": { "x": 188, "y": 228 },
      "pixels_per_tile": 200.0,
      "hitbox": { "x": 48, "y": 50, "w": 280, "h": 178 },
      "animations": {
        "idle": { "path": "enemies/slime_green", "frames": 30, "fps": 24.0, "looping": true }
      }
    }
  }
}
```

- **Sprite** : un « objet » dessinable (personnage, ennemi, plante) regroupant des animations qui
  partagent la même taille de frame et le même pivot. Les ids des slimes sont identiques aux
  `kind` d'entité de `mw_level`.
- Coordonnées en **pixels de la frame source**, origine en haut à gauche, **y vers le bas**.
- `pivot` : point placé à la position de l'entité. Pieds pour les personnages et ennemis, base
  de la tige pour les plantes, **haut** pour `plant_8_poison`, qui pend du plafond. Il est commun à
  toutes les animations d'un sprite pour éviter que le sprite saute d'une animation à l'autre.
- `pixels_per_tile` : pixels source par tile de niveau ; échelle de rendu = `tile_size / pixels_per_tile`
  (wizard : 140 → ~2 tiles de haut ; plantes : 160).
- `hitbox` (optionnelle) : boîte de collision **suggérée** (wizard, slimes). La valeur de gameplay
  appartient aux `Rules` de la sim, qui peut partir de celle-ci.
- `animations.<nom>` : `path` (dossier relatif à `assets/`), `frames`, `fps`, `looping` (sinon la
  dernière frame est tenue). Frames : `<path>/0000.png` … (`AnimationDef::frame_path`).

| Sprite | Animations |
|---|---|
| `blue_wizard` | `idle` (12 fps), `walk` (24), `jump` (12, non bouclée), `dash2`, `dash3`, `dash_effect` (30, non bouclées) |
| `slime_green`, `slime_orange` | `idle` (24) |
| `plant_1..7`, `blue_flower_1/2` | `idle` (30) |
| `plant_8_poison`, `plant_wind_1` | `idle` (24) |
| `plant_jump` | `idle` = `plants/plant_jump2` (24, ondulation légère), `bounce` = `plants/plant_jump` (30, non bouclée : le champignon s'étire, à jouer quand on rebondit dessus) |

Les pivots et hitboxes ont été mesurés sur l'enveloppe opaque des frames, puis vérifiés en les
dessinant par-dessus les images.

## Mémoire et rognage
Les frames ne sont **pas rognées** : les PNG sont conservés tels quels. Mesure faite pendant #1 :
toutes les frames décodées en RGBA pleine résolution occupent **~1011 Mio**. Rognées à l'union
opaque de chaque animation, elles occupent ~471 Mio. Comme les sprites s'affichent bien plus petits
que leur résolution source, une réduction (×4) est le vrai levier. Voir l'issue #30 (outil `xtask`
qui génère des frames rognées et réduites avec un manifest cohérent). D'ici là, le rendu ne doit
charger que les animations des sprites présents dans le niveau.
