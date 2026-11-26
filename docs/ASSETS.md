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
Les métadonnées (fps, pivot, hitbox, découpage des planches) vivent dans `assets/manifest.json`
et `assets/tilesets/mossy/atlas.json` (issues M0).
