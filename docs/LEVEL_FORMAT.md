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
  "legend": { "#": "mossy_ground", "=": "mossy_ground", "^": "thorns_tile" },
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
| `legend` | objet `caractère → id` | non | Associe chaque caractère de tile à un id de pièce ou de famille de terrain auto-tilé (voir `docs/ASSETS.md`). |
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
| `dash_speed` | nombre > 0 | `650.0` | Vitesse du dash horizontal (u/s). |
| `dash_duration` | secondes > 0 | `0.15` | Durée du dash, arrondie en ticks, au moins 1 (9 par défaut). |
| `dash_cooldown` | secondes ≥ 0 | `0.3` | Intervalle minimum entre deux départs de dash (18 ticks). |
| `air_jumps` | entier non signé | `1` | Sauts supplémentaires avant de retoucher le sol (`0` les supprime). |
| `wall_slide_speed` | nombre ≥ 0 | `100.0` | Vitesse de chute max en poussant contre un mur. |
| `wall_jump_speed_x` / `wall_jump_speed_y` | nombre > 0 | `280.0` / `600.0` | Impulsion loin du mur / contre la gravité (u/s). |
| `wall_jump_lock` | secondes ≥ 0 | `0.1` | Durée sans contrôle horizontal après le wall-jump (6 ticks). |

Mécaniques disponibles : `movement` (course), `jump` (saut), `dash`, `double_jump`, `wall_jump` et `melee`.
Les trois dernières sont enregistrées mais **désactivées par défaut**. Exemple :
`"mechanics": ["movement", "jump", "dash", "double_jump", "wall_jump"]`.
`melee` active le combat de #7. Toutes les durées sont arrondies au tick le plus proche.

- `dash` : nouveau front de `DASH`, axe horizontal ou orientation précédente, direction figée
  pendant le dash. Gravité suspendue, collision solide conservée. Une charge avant de retoucher
  le sol, cooldown compté depuis le départ ; garder le bouton enfoncé ne relance pas le dash.
  À la fin, la vitesse X est ramenée au plus à `run_speed` (0 sans `movement`).
- `double_jump` : nouveau front de `JUMP` en l'air, jusqu'à `air_jumps` fois. Un saut au sol ou
  en coyote time ne consomme pas de charge. Recharge au sol, même avec gravité inversée.
- `wall_jump` : pousser vers un mur limite la chute à `wall_slide_speed`. Un nouveau front de
  `JUMP` en contact avec le mur propulse à l'opposé et contre la gravité. Le verrou horizontal
  évite que l'input maintenu annule immédiatement l'impulsion. Pas de recharge aérienne au mur.

Les sauts aériens et muraux peuvent fonctionner sans `jump` ; le saut variable et le buffer
restent des fonctionnalités de `jump`. Sur un même tick : saut au sol/coyote → wall-jump → saut
aérien ; seul le premier possible est exécuté, puis un départ de dash prend la priorité sur la
vitesse. Pendant un dash en cours, les sauts sont ignorés.

### Paramètres de combat (`Rules`)
| Clé | Type | Défaut | Description |
|---|---|---|---|
| `player_health` | entier > 0 | `5` | PV maximum/initial des nouveaux joueurs. |
| `melee_damage` | entier > 0 | `1` | Dégâts d'un coup. |
| `melee_reach` | nombre > 0 | `32.0` | Portée depuis le bord de la hitbox du corps, unités monde. |
| `melee_thickness` | nombre > 0 | `32.0` | Épaisseur du coup sur l'autre axe, unités monde. |
| `melee_duration` | secondes > 0 | `0.1` | Durée active, arrondie en ticks (minimum 1). |
| `melee_cooldown` | secondes ≥ 0 | `0.25` | Délai entre démarrages de coups (minimum 1 tick). |
| `damage_invincibility` | secondes ≥ 0 | `0.5` | Invincibilité après un impact accepté. |
| `hit_stun` | secondes ≥ 0 | `0.1` | Durée pendant laquelle le knockback prime sur les déplacements. |
| `knockback_x` / `knockback_y` | nombre ≥ 0 | `260.0` / `180.0` | Vitesse imposée pendant le hit-stun : X s'éloigne de la source, Y contre la gravité. |
| `pogo_speed` | nombre ≥ 0 | `420.0` | Vitesse de rebond d'un coup vers le bas contre ennemi/hazard. |

**Repères de level design (règles par défaut)** : un saut monte de 4 tiles au maximum. Prévoir
**3 tiles au plus** entre deux surfaces pour un saut confortable. En élan, on franchit ~4 tiles
de vide à niveau égal, davantage en descendant. Le joueur passe sous un plafond à 2 tiles.
`crates/mw_sim/tests/tutorial.rs` montre comment vérifier un parcours par un test headless.

Le combat est optionnel : ajouter `melee` à `rules.mechanics` pour l’activer.

Cette table est mise à jour par chaque issue qui ajoute un champ à `Rules`.

### Calques (`Layer`)
| Champ | Description |
|---|---|
| `name` | Nom unique non vide. |
| `kind` | `solid` (collision), `decor` (visuel seulement) ou `hazard` (blesse au contact). |
| `tiles` | Exactement `size.h` chaînes de `size.w` caractères. **La première chaîne est la ligne du haut**, pour que le fichier ressemble au niveau. |

Caractères : `.` = vide (réservé, interdit dans `legend`) ; tout autre caractère doit figurer dans
`legend` et être de l'ASCII imprimable sans espace.

Le calque détermine la collision : `=` utilise le terrain auto-tilé `mossy_ground` pour les
plateformes d'une tile de haut. Un calque `hazard` peut répéter `thorns_tile`, une découpe
d'épines dont le dessin tient dans une cellule (voir `docs/ASSETS.md`). La case entière est
la zone de danger ; le canal alpha du sprite ne définit pas la collision. Dans le tutorial,
le chemin prévu reste P1 → P2 → P3 au-dessus de la fosse ; les plateformes sont encore solides
par le dessous, comportement suivi séparément par #34.

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

`Level::validate_with_tile_ids(&BTreeSet<String>)` et
`LevelBundle::validate_with_tile_ids(&BTreeSet<String>)` ajoutent la validation de **toutes**
les entrées de légende, même inutilisées. L'appelant fournit les ids de pièces **et** les
familles de terrain auto-tilé de son atlas. `mw_level` n'a aucune dépendance d'exécution à
`mw_core` ni à un atlas particulier ; `validate()` reste disponible pour la seule structure.
Les diagnostics d'ids inconnus suivent les erreurs structurelles (et les scripts manquants
pour un bundle), dans l'ordre des caractères de légende. Exemple avec `TilesetAtlas` :

```rust,ignore
let tile_ids = atlas.pieces.keys().cloned().chain(
    atlas.pieces.values().filter_map(|p| {
        p.autotile.as_ref().map(|a| a.terrain.clone())
    }),
).collect();
let errors = bundle.validate_with_tile_ids(&tile_ids);
```

| `code` | Cause |
|---|---|
| `invalid_id` | `id` vide ou hors `[a-z0-9_]`. |
| `invalid_size` / `invalid_tile_size` | Taille de grille nulle / `tile_size` = 0. |
| `invalid_legend_char` / `empty_tile_id` | Caractère de légende interdit / id de tile vide. |
| `unknown_tile_id` | Id de légende absent de la palette fournie (`ch`, `id`). Seulement avec `validate_with_tile_ids`. |
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
