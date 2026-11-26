# Format de niveau — schéma v1

Implémenté par la crate `mw_level` (sans Bevy), partagée par le jeu, l'éditeur et le serveur MCP.
Exemple complet : [`levels/tutorial/level.json`](../levels/tutorial/level.json).

```
levels/<id>/
  level.json
  scripts/main.rhai      # optionnel, plusieurs fichiers possibles (scripts/**/*.rhai)
```

## Exemple
```json
{
  "schema_version": 1,
  "id": "forest_01",
  "name": "Forêt moussue",
  "size": { "w": 8, "h": 4 },
  "tile_size": 32,
  "legend": { "#": "mossy_ground", "=": "mossy_platform", "^": "mossy_thorns" },
  "layers": [
    {
      "name": "ground",
      "kind": "solid",
      "tiles": [
        "#......#",
        "#..===.#",
        "#......#",
        "###..###"
      ]
    },
    { "name": "thorns", "kind": "hazard", "tiles": ["........", "........", "........", "...^^..."] }
  ],
  "spawns": [ { "x": 1.5, "y": 1.0 } ],
  "entities": [ { "kind": "slime_green", "x": 4.5, "y": 3.0, "props": { "patrol": 2.0 } } ],
  "rules": { "gravity": -1800.0, "mechanics": ["jump", "dash"] },
  "scripts": ["scripts/main.rhai"]
}
```

## Champs
| Champ | Type | Requis | Description |
|---|---|---|---|
| `schema_version` | entier | oui | Version du format. Actuelle : **1**. |
| `id` | chaîne | oui | `[a-z0-9_]+`, identique au nom du dossier sous `levels/`. |
| `name` | chaîne | oui | Nom affiché. |
| `size` | `{ "w", "h" }` | oui | Taille de la grille **en tiles** (≥ 1×1). |
| `tile_size` | entier | non (32) | Unités monde par tile. |
| `legend` | objet `caractère → id` | non | Associe chaque caractère de tile à un id de pièce du tileset (voir `docs/ASSETS.md`). |
| `layers` | tableau de `Layer` | oui | Calques, dessinés dans l'ordre (premier = le plus au fond). |
| `spawns` | tableau de `{ "x", "y" }` | oui | Points d'apparition des joueurs (au moins un). |
| `entities` | tableau de `EntityDef` | non | Entités placées (ennemis…). |
| `rules` | objet `clé → valeur JSON` | non | Surcharges des `Rules` de la simulation (voir ci-dessous). Les clés sont vérifiées par `mw_sim` au chargement, pas par `mw_level`. |
| `scripts` | tableau de chemins | non | Scripts Rhai, relatifs au dossier du niveau, de la forme `scripts/**/*.rhai`. |

Les champs inconnus sont **refusés** au chargement (une faute de frappe ne passe pas inaperçue).

### Règles (`rules`)
Surcharges des valeurs par défaut de `mw_sim::Rules`. Une clé inconnue, un mauvais type ou une
mécanique non implémentée fait **refuser le niveau** au chargement. Distances en unités monde
(1 tile = `tile_size`), vitesses en unités/s.

| Clé | Type | Défaut | Description |
|---|---|---|---|
| `gravity` | nombre | `-1800.0` | Accélération verticale (u/s²). Négative = vers le bas ; positive = gravité inversée. |
| `max_fall_speed` | nombre > 0 | `900.0` | Vitesse maximale dans le sens de la gravité (u/s). |
| `mechanics` | liste de chaînes | `["movement", "jump"]` | Mécaniques actives (voir ci-dessous). Remplace la liste entière. |
| `player_width` / `player_height` | nombre > 0 | `24.0` / `56.0` | Hitbox du joueur (0,75 × 1,75 tile). |
| `run_speed` | nombre ≥ 0 | `220.0` | Vitesse de course max (u/s, ~6,9 tiles/s). |
| `ground_accel` / `ground_decel` | nombre > 0 | `2400.0` / `3200.0` | Accélération / freinage au sol (u/s²). |
| `air_accel` / `air_decel` | nombre > 0 | `1500.0` / `600.0` | Accélération / freinage en l'air (u/s²) ; faible freinage = élan conservé. |
| `jump_height` | nombre ≥ 0 | `128.0` | Hauteur d'un saut complet (u, 4 tiles), garantie quelle que soit la gravité. |
| `jump_cut` | 0..=1 | `0.4` | Facteur appliqué à la vitesse montante si on relâche tôt (saut à hauteur variable). |
| `coyote_time` | secondes ≥ 0 | `0.1` | Délai pendant lequel on peut encore sauter après avoir quitté un rebord. |
| `jump_buffer` | secondes ≥ 0 | `0.1` | Un saut pressé juste avant d'atterrir part à l'atterrissage. |

Mécaniques disponibles : `movement` (course), `jump` (saut). Les suivantes (`double_jump`, `dash`,
`wall_jump`, `melee`…) arrivent avec #6 et #7.

**Repères de level design (règles par défaut)** : un saut monte de 4 tiles au maximum. Prévoir
**3 tiles au plus** entre deux surfaces pour un saut confortable. En élan, on franchit ~4 tiles
de vide à niveau égal, davantage en descendant. Le joueur passe sous un plafond à 2 tiles.
`crates/mw_sim/tests/tutorial.rs` montre comment vérifier un parcours par un test headless.

Cette table est mise à jour par chaque issue qui ajoute un champ à `Rules`.

### Calques (`Layer`)
| Champ | Description |
|---|---|
| `name` | Nom unique non vide. |
| `kind` | `solid` (collision), `decor` (visuel seulement) ou `hazard` (blesse au contact). |
| `tiles` | Exactement `size.h` chaînes de `size.w` caractères. **La première chaîne est la ligne du haut**, pour que le fichier ressemble au niveau. |

Caractères : `.` = vide (réservé, interdit dans `legend`) ; tout autre caractère doit figurer dans
`legend` et être de l'ASCII imprimable sans espace.

### Entités (`EntityDef`)
| Champ | Description |
|---|---|
| `kind` | Type d'entité. Connus en v1 : `slime_green`, `slime_orange` (`mw_level::KNOWN_ENTITY_KINDS`). |
| `x`, `y` | Position (voir Coordonnées). |
| `props` | Paramètres propres au type (objet libre, défaut `{}`). |

## Coordonnées
- Unité : la **tile** (nombres réels autorisés). Multiplier par `tile_size` pour obtenir les unités monde.
- Origine en **bas à gauche** de la grille, **Y vers le haut**. La tile `(i, j)` couvre `[i, i+1) × [j, j+1)`.
- La ligne `tiles[r]` correspond à `y = size.h - 1 - r` (`Layer::tile_at(x, y)` fait la conversion).
- Un spawn ou une entité est ancré au **milieu du bas** de sa hitbox (ses pieds) : pour poser un
  joueur sur le sol dont le dessus est à `y = 2`, au centre de la colonne 3, écrire `{ "x": 3.5, "y": 2.0 }`.
- Les positions doivent être dans `[0, w] × [0, h]`. Un spawn ne doit pas être dans une tile `solid`.

## Validation
`Level::validate()` (structure) et `LevelBundle::validate()` (structure + présence des sources de
scripts) renvoient **toutes** les erreurs trouvées, dans un ordre déterministe. Chaque erreur se
sérialise en JSON avec un champ `code` (utilisé par l'éditeur et le MCP), par exemple :
```json
{ "code": "unknown_tile", "layer": "ground", "ch": "?", "row": 0, "col": 2 }
```
| `code` | Cause |
|---|---|
| `invalid_id` | `id` vide ou hors `[a-z0-9_]`. |
| `invalid_size` / `invalid_tile_size` | Taille de grille nulle / `tile_size` = 0. |
| `invalid_legend_char` / `empty_tile_id` | Caractère de légende interdit / id de tile vide. |
| `empty_layer_name` / `duplicate_layer_name` | Nom de calque vide / répété. |
| `wrong_row_count` / `wrong_row_width` | Nombre de lignes ≠ `h` / longueur de ligne ≠ `w`. |
| `unknown_tile` | Caractère absent de `legend` (signalé une fois, à sa première occurrence). |
| `no_spawn` / `spawn_out_of_bounds` / `spawn_in_solid` | Pas de spawn / spawn hors niveau / dans le sol. |
| `unknown_entity_kind` / `entity_out_of_bounds` | `kind` inconnu / entité hors niveau. |
| `invalid_script_path` / `duplicate_script` / `missing_script` | Chemin non `scripts/**/*.rhai` (ou avec `..`) / listé deux fois / fichier absent. |

## Sauvegarde canonique
`LevelBundle::save` écrit du JSON indenté (2 espaces), clés dans l'ordre des champs ci-dessus,
`legend`/`rules`/`props` triés par clé, une ligne de tiles par ligne, retour à la ligne final.
Charger puis sauvegarder un fichier canonique donne **exactement** les mêmes octets ; un test vérifie
que tous les niveaux de `levels/` sont valides et canoniques.

## Versions et migrations
Au chargement, le JSON brut est mis à niveau version par version (`mw_level::migrate`) avant d'être
désérialisé. Pour une évolution incompatible :
1. incrémenter `CURRENT_SCHEMA_VERSION` ;
2. ajouter la fonction `vN → vN+1` à `MIGRATIONS` (elle met aussi à jour `schema_version`) et son test ;
3. mettre à jour ce document ;
4. re-sauvegarder les niveaux de `levels/`.

Une version absente, nulle ou plus récente que celle du build est refusée.
