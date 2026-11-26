# Mosswake — instructions pour agents IA

Mosswake est un jeu de plateforme/action 2D **multijoueur en réseau local** (style metroidvania),
écrit en **Rust** avec **Bevy**, réseau **UDP** (serveur autoritaire), niveaux en **JSON** et
**scripts Rhai embarqués dans chaque niveau** pour changer radicalement les mécaniques.
Un **éditeur** in-game et un **serveur MCP** (`mosswake-mcp`) permettent aux humains et aux IA
de créer des niveaux. Le projet est entièrement développé par des agents : lis ce fichier en entier.

## Avant de coder
1. Lis `CONTRIBUTING.md` (workflow issue → branche → PR **obligatoire**).
2. Lis `docs/ARCHITECTURE.md` (frontières des crates, principes non négociables).
3. Selon la zone : `docs/LEVEL_FORMAT.md`, `docs/SCRIPTING_API.md`, `docs/NETCODE.md`, `docs/MCP.md`, `docs/ASSETS.md`.

## Commandes
```sh
cargo build                                   # tout le workspace
cargo test                                    # tous les tests
cargo clippy --all-targets -- -D warnings     # doit être propre
cargo fmt                                     # avant chaque commit
cargo run -p mosswake -- <subcommand>         # jeu (host | join <addr> | edit <level> | server --headless)
cargo run -p mosswake-mcp                     # serveur MCP (stdio)
```
Toolchain : `rust-toolchain.toml` (stable). Linux : `libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev pkg-config`.

## Carte du repo
| Chemin | Rôle |
|---|---|
| `crates/mw_core` | Types partagés, constantes, IDs, maths, RNG déterministe. Aucune dépendance lourde. |
| `crates/mw_level` | Format de niveau (serde JSON), `schema_version`, validation, migrations. **Pas de Bevy.** |
| `crates/mw_sim` | Simulation gameplay déterministe, headless (Bevy `MinimalPlugins`, pas de rendu). Physique, joueurs, ennemis, mécaniques, `Rules`. |
| `crates/mw_script` | Moteur Rhai sandboxé, hooks, API exposée aux scripts. |
| `crates/mw_net` | Transport UDP, messages versionnés, découverte LAN. |
| `crates/mw_render` | Rendu client uniquement : sprites, animations, tilemap, caméra, parallax, HUD. |
| `crates/mw_editor` | Éditeur in-game (bevy_egui). |
| `bins/mosswake` | Exécutable jeu (client, host, serveur headless, éditeur). |
| `bins/mosswake-mcp` | Serveur MCP stdio pour agents (manipule les niveaux via `mw_level`). |
| `assets/` | Sprites (frames PNG), tilesets. Voir `docs/ASSETS.md`. |
| `levels/<id>/` | `level.json` + `scripts/*.rhai`. |

## Règles non négociables
- **Le serveur fait autorité.** Toute logique de gameplay vit dans `mw_sim` (+ `mw_script`), jamais dans `mw_render`/`mw_editor`.
- **Déterminisme** : tick fixe 60 Hz, pas de `rand` non seedé, pas de temps réel (`Instant::now`) dans la sim, itérations ordonnées (pas de `HashMap` itérée pour la logique → `BTreeMap`/`IndexMap`).
- **Aucune constante de gameplay en dur** : vitesse, gravité, sauts, dégâts… vont dans la ressource `Rules` (chargée depuis le niveau, modifiable par script). Chaque mécanique (dash, double saut, wall-jump…) est un **plugin activable** par le niveau.
- **Les scripts Rhai tournent côté serveur uniquement**, sandboxés (limite d'opérations, pas d'I/O). Toute nouvelle fonction exposée à Rhai doit être documentée dans `docs/SCRIPTING_API.md` et testée.
- **Format de niveau** : tout changement incompatible → bump `schema_version` + migration + mise à jour de `docs/LEVEL_FORMAT.md` et des niveaux dans `levels/`.
- **Protocole réseau** : tout changement de message → bump `PROTOCOL_VERSION`.
- Respecte les frontières : `mw_level` et `mw_core` ne dépendent pas de Bevy ; `mw_sim` ne dépend pas de `mw_render`.
- Pas d'`unwrap()` hors tests/prototypes évidents ; erreurs via `thiserror` (libs) / `anyhow` (bins).
- Dépendances : déclare la version dans `[workspace.dependencies]` du `Cargo.toml` racine, puis `foo.workspace = true` dans la crate. Vérifie la compatibilité des versions (ex. `bevy_egui` ↔ `bevy`).
- Chaque PR ajoute/maintient des **tests** (unitaires, ou tests headless de `mw_sim` qui simulent N ticks avec des inputs scriptés).
- Reste dans le périmètre de ton issue. Si tu découvres un travail nécessaire hors périmètre, **crée une nouvelle issue** (template Task) au lieu de l'implémenter.

## Vérifier son travail
- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` doivent passer (la CI fait la même chose).
- Pour le gameplay, préfère un test headless dans `mw_sim` à une vérification visuelle.
- Si tu touches au rendu, lance le jeu et décris ce que tu as vu dans la PR.

## Langue
Code, identifiants et commentaires en anglais. Issues/PR en français ou anglais.
