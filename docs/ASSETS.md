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

### Hazards dans une grille
`thorns_tile` est la pièce dédiée aux cases `hazard` : découpe centrale de 512×316 px dans
`decorations_and_hazards.png` à `(3278, 1939)`, au sein du dessin `thorns_small_long`.
À 512 px/tile, son dessin fait exactement une tile de large et moins d'une tile de haut ;
son emprise est 1×1, ancrage `bottom`, `collision: false` (pas de mur solide). Le calque
`hazard` définit la case dangereuse, indépendamment des pixels opaques. Répéter cette pièce
n'étend donc pas le dessin sur les cellules voisines. Les lianes complètes restent disponibles
comme pièces libres ; aucun PNG source n'est modifié. Le chevauchement des deux rectangles
est volontaire et testé par inclusion, seule exception au test de séparation des pièces libres.

Les légendes utilisent les ids de `pieces` ou les familles `autotile.terrain` (ex.
`mossy_ground`). Les ids `mossy_platform` et `mossy_thorns` n'existent pas : le tutorial emploie
`mossy_ground` pour `#` et `=`, puis `thorns_tile` pour `^`. Le test de `mw_level` vérifie chaque
niveau avec cette palette réelle via `validate_with_tile_ids`.

### Format JSON
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
Les PNG et le manifest de `assets/` sont les **sources intactes**. La préparation des dérivés
est explicite ; elle ne se lance pas pendant `cargo build` et ne remplace pas les sources :

```sh
cargo xtask build-assets
cargo xtask build-assets --scale 2
cargo xtask build-assets --scale 4 --source assets --output target/generated-assets
```

Le facteur est un entier de 1 à 64 (défaut 4 ; 1 = rognage seul). Les chemins par défaut
sont relatifs à la racine du workspace ; les chemins explicitement fournis sont relatifs au
dossier courant. `cargo xtask --help` affiche l'aide. Seule la dépendance `image` avec son
codec PNG est utilisée, sans fenêtre ni GPU.

### Sorties et versionnement
`target/generated-assets/` est ignoré par Git via `/target` : on versionne l'outil, son lockfile
et les sources, et on régénère les sorties avant de les distribuer. Il contient :

- `manifest.json`, au format v1, chargé avec `AnimationManifest::load(output)` et validé contre
  toutes les frames générées ;
- `frames/<index-sprite>/<index-animation>/NNNN.png` : indices stables selon l'ordre des ids
  et noms dans le manifest ; les noms de sprite restent les mêmes dans le manifest ;
- `build-report.json` : facteur, nombre de frames, rectangles de rognage en coordonnées source
  et octets RGBA8 avant/après (aucun chemin absolu ni horodatage).

Ce dossier est une racine **d'animations**, pas un remplacement complet de `assets/` : les
tilesets et leur atlas restent dans les sources. Un consommateur des dérivés doit charger
ce manifest et résoudre ses chemins de frames contre le même dossier généré ; changer
seulement le manifest en conservant l'ancienne racine de frames ne fonctionne pas.

On peut relancer la commande dans le même dossier : la génération et la validation se font
dans un répertoire temporaire voisin, puis le répertoire précédent est remplacé, sans frames
périmées. Une erreur de lecture/validation laisse les anciennes sorties utilisables. L'outil
refuse de remplacer un dossier qui ne porte pas son rapport de génération, ainsi que toute
sortie recouvrant les sources (parents, enfants ou liens symboliques).

### Géométrie et reproductibilité
L'enveloppe inclut chaque pixel de chaque frame dont l'alpha est non nul. Le manifest v1
partage `frame_size` et `pivot` entre toutes les animations d'un sprite : leurs unions opaques
sont donc réunies dans **un cadre commun par sprite**, plutôt que de créer un format incompatible
avec des repères par animation. Cela conserve les changements d'animation sans décalage.
Le pivot et la hitbox sont inclus même s'ils dépassent les pixels peints.

Le cadre est aligné sur un multiple du facteur **relativement au pivot**. Un petit ajout de
transparence est possible au bord du fichier : aucune taille n'est étirée pour faire rentrer
un rectangle indivisible. Pour un facteur `s` et une origine de cadre `(cx, cy)` :

- `frame_size = (largeur_cadre / s, hauteur_cadre / s)` ;
- `pivot = ((pivot_source.x - cx) / s, (pivot_source.y - cy) / s)`, exact en pixels entiers ;
- `pixels_per_tile = pixels_per_tile_source / s` : le pivot reste exactement à sa position monde ;
- les bords de hitbox sont arrondis vers l'extérieur, avec une extension inférieure à un pixel
  généré par côté. La hitbox suggérée ne rétrécit jamais ; les `Rules` de simulation restent inchangées.

Les images sont réduites par Lanczos3 sur couleurs prémultipliées par l'alpha, puis reconverties
en RGBA8, pour éviter les contours assombris. Le manifest conserve fps, bouclage et nombre de
frames. Une animation entièrement transparente garde un cadre valide incluant son pivot.
L'ordre des opérations, le JSON et l'encodage PNG sont déterministes avec le `Cargo.lock`
versionné ; les tests comparent les octets de deux générations et d'une régénération.

### Mesure
Le rapport additionne `largeur × hauteur × 4` pour toutes les frames : taille des pixels RGBA8
décodés, hors mipmaps, alignement GPU et compression PNG. Le rendu doit encore ne charger que
les animations des sprites présents dans le niveau.

Mesure sur le manifest versionné (886 frames d'animation ; les 6 autres PNG sont les planches
du tileset, non transformées), avec `cargo xtask build-assets --scale 4` :

| Données | Octets RGBA8 | Mio |
|---|---:|---:|
| Frames source | 1 060 052 896 | 1 010,95 |
| Frames générées | 32 048 976 | 30,56 |

Réduction de **96,98 %**, environ **33 fois moins** de pixels à charger. L'estimation initiale
de ~471 Mio après simple rognage utilisait des enveloppes indépendantes par animation ; le cadre
commun par sprite et la conservation des repères expliquent un peu de transparence supplémentaire.
La génération réelle vérifie le manifest de sortie ; une comparaison SHA-256 des PNG sources et
du manifest avant/après a confirmé qu'ils restent identiques.

### Réduction au chargement
Depuis #10, le rendu réduit aussi chaque texture au chargement à environ 64 px par tile
(`mw_render::textures`) avant l'envoi au GPU, et ne charge que les sprites présents dans le
niveau. La VRAM est donc déjà maîtrisée avec les sources. Les dérivés préparés par xtask
permettent surtout d'éviter le décodage des PNG pleine taille et de réduire la RAM de pointe
au démarrage, quand le consommateur utilise leur manifest et leur racine de frames.
