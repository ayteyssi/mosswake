# Serveur MCP d'édition de niveaux

`mosswake-mcp` expose 11 outils MCP en **stdio**, sans Bevy. Il utilise `rmcp` et la validation
commune de `mw_level`. Les appels d'une même instance sont sérialisés pour éviter de perdre
une modification lors de deux appels concurrents.

## Démarrage

Depuis la racine du dépôt :
```sh
cargo run --quiet -p mosswake-mcp
```
La configuration `.mcp.json` lance cette commande. Configurer le répertoire de travail du client
MCP sur la racine du dépôt. Pour travailler ailleurs, fournir des chemins explicites :
```sh
cargo run --quiet -p mosswake-mcp -- --levels-dir /chemin/levels --tileset-dir /chemin/assets/tilesets/mossy
```
`stdout` est réservé au protocole JSON-RPC, les diagnostics vont sur `stderr`.
Le serveur peut créer le dossier `levels` ; l'atlas doit déjà exister.

## Outils

Tous les outils renvoient du JSON dans `structuredContent` ainsi qu'un contenu texte MCP.
Les erreurs de manipulation donnent `isError: true` et `{ "error": "message" }`.
Les erreurs de validation incluent les codes de `mw_level` (voir `LEVEL_FORMAT.md`).

| Outil | Arguments / résultat |
|---|---|
| `list_levels` | Sans arguments. Liste triée par id, nom, taille et erreurs ; un niveau illisible est signalé sans cacher les autres. |
| `list_palette` | Sans arguments. `entity_kinds`, ids de `terrains`, et `pieces` de l'atlas avec type, taille et collision. |
| `create_level` | `id`, `name`, `width`, `height`, `spawns?`. Crée un calque `ground` solide vide. Spawn par défaut `(0.5, 0)`. Refuse d'écraser un niveau existant. |
| `get_level` | `id`. JSON complet, `grids_top_to_bottom` lisibles et `indexed_entities` pour identifier les entités à supprimer. |
| `set_tiles` | `id`, `layer`, `x`, `y`, `width`, `height`, `tile`, `tile_id?`, `kind?`. Remplit un rectangle ; hauteur/largeur 1 pour une ligne. `.` gomme. `tile_id` associe le caractère à un id de palette. Un nouveau calque nécessite `kind` (`solid`, `decor`, `hazard`). |
| `place_entity` | `id`, `kind`, `x`, `y`, `props?` (objet). Ajoute une entité, renvoie son `index`. |
| `remove_entity` | `id`, `index`. Supprime par index, **les indices suivants se décalent** : relire après suppression. |
| `set_rules` | `id`, `rules` (objet), `replace?` (défaut faux). Fusionne les clés ou remplace toutes les surcharges. |
| `write_script` | `id`, `path`, `source`. Crée/remplace le fichier et l'ajoute automatiquement à `level.scripts` s'il n'y figure pas. |
| `read_script` | `id`, `path`. Renvoie la source d'un script référencé par le niveau. |
| `validate_level` | `id`. `{ "valid": bool, "errors": [...] }`, sans écrire. |

Les schémas MCP documentent chaque champ et refusent les champs inconnus.
Les ids sont des noms `[a-z0-9_]+` de 1 à 64 caractères, jamais des chemins.
Les chemins de scripts sont relatifs (`scripts/**/*.rhai`), sans `..` ni liens symboliques.
Une création est limitée à 262144 cellules et un script à 1 Mio.

## Coordonnées et exemple

Toutes les coordonnées d'édition sont en **tiles**, origine en bas à gauche, **Y vers le haut**.
`get_level` affiche les chaînes de grille dans l'ordre du fichier : **haut vers bas**.
Les positions de spawns et d'entités représentent les pieds (milieu du bas de la boîte).

Séquence d'appels `tools/call` (les objets ci-dessous sont leurs `arguments`) :
```json
{"id":"mcp_demo","name":"Démo MCP","width":16,"height":8,"spawns":[{"x":1.5,"y":1.0}]}
```
Appeler `create_level`, puis `list_palette` pour choisir les ids, puis `set_tiles` :
```json
{"id":"mcp_demo","layer":"ground","x":0,"y":0,"width":16,"height":1,"tile":"#","tile_id":"mossy_ground"}
```
Appeler `place_entity`, puis `write_script` :
```json
{"id":"mcp_demo","kind":"slime_green","x":8.5,"y":1.0,"props":{"patrol":2.0}}
```
```json
{"id":"mcp_demo","path":"scripts/main.rhai","source":"fn on_level_start() {}\n"}
```
Terminer avec `get_level` et `validate_level` (`{"id":"mcp_demo"}`).

## Validation et écritures

Toutes les écritures valident le **bundle résultant**, avec `LevelBundle::validate()`, avant de
publier des fichiers. Ainsi, peindre du solide sur un spawn est refusé et laisse le JSON intact.
Un niveau invalide reste lisible et peut être réparé si l'opération le rend valide (ex. réécrire un
script manquant). Les ids de légende contre l'atlas relèvent de #32 ; actuellement la validation
commune vérifie la présence des caractères dans la légende, pas l'existence des assets.

Les écritures sont préparées dans des fichiers temporaires du même répertoire, synchronisées,
puis publiées par remplacement atomique de **chaque fichier** ; les scripts précèdent
`level.json` pour éviter de référencer un fichier pas encore créé. La création d'un niveau publie
un répertoire temporaire par un seul renommage. Une mise à jour de plusieurs fichiers n'est pas
une transaction globale en cas d'erreur disque pendant les renommages. Éviter plusieurs
instances d'édition sur le même dossier : le verrou sérialise les appels d'une instance seulement.

Le JSON sauvegardé est canonique (`mw_level`). Les anciens scripts non référencés sont laissés
sur disque. Les changements de `Rules` sont validés par la simulation au chargement, et les
sources Rhai par `mw_script`, pas par ce serveur d'édition structurelle.
`simulate_level` et `render_preview` seront ajoutés dans #27.

## Tests

```sh
cargo test -p mosswake-mcp
```
Les tests appellent les handlers sans transport : cycle d'édition complet, coordonnées,
rectangles/effacement, entités et indices, règles fusion/remplacement, scripts imbriqués,
validation et réparation d'un script manquant, refus des modifications invalides sans changement
des octets, confinement des chemins/liens symboliques, et présence/descriptions des 11 schémas.
