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

Tilesets (`tilesets/mossy/`, grandes planches peintes, **pas** une grille régulière de petites tiles) :
| Fichier | Taille |
|---|---|
| `tile_set.png` | 3584×3584 (blocs de terrain moussus, ~grille de 512 px) |
| `floating_platforms.png` | 2048×2048 |
| `decorations_and_hazards.png` | 4096×4096 |
| `background_decoration.png` | 4096×4096 |
| `hanging_plants.png` | 2048×2948 |
| `mossy_hills.png` | 2048×2048 (parallax) |

Les sprites sont haute résolution : ils seront affichés à l'échelle (ex. wizard ≈ 2 tiles de haut).
Le découpage des planches vit dans `assets/tilesets/mossy/atlas.json` (issue #2).

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
