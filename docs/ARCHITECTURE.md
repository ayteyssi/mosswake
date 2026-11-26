# Architecture

## Vue d'ensemble
```
           ┌──────────────── bins/mosswake ────────────────┐
 inputs ─► │ client: mw_render + prédiction (mw_sim)       │ ◄─ snapshots ─┐
           └──────────────────────┬────────────────────────┘               │
                                  │ inputs (UDP, mw_net)                   │
           ┌──────────────────────▼────────────────────────┐               │
           │ serveur autoritaire: mw_sim + mw_script        │ ──────────────┘
           │ (peut tourner dans le process de l'hôte)       │
           └──────────────────────┬────────────────────────┘
                                  │ charge
                         levels/<id>/level.json + scripts/*.rhai  ◄── mw_editor / mosswake-mcp (mw_level)
```

## Simulation (`mw_sim`)
- Bevy ECS, `FixedUpdate` à **60 Hz**. Fonctionne sans fenêtre (`MinimalPlugins`) pour serveur headless et tests.
- Entrée de la sim : `PlayerInput` par joueur et par tick (bitflags + axes). Sortie : état des entités.
- **`Rules`** (ressource) : *tous* les paramètres de gameplay (gravité, vitesses, hauteurs de saut,
  nb de sauts, dash, wall-jump, PV, dégâts, PvP on/off, friction…). Valeurs par défaut dans le code,
  surchargées par `level.json` → `rules`, puis modifiables à chaud par les scripts.
- **Mécaniques = plugins activables** : `MechanicsPlugin` enregistre chaque mécanique
  (`movement`, `jump`, `double_jump`, `dash`, `wall_jump`, `melee`, …) avec un identifiant ;
  le niveau liste celles qui sont actives (`rules.mechanics`). Ajouter une mécanique =
  nouveau module + entrée au registre + paramètres dans `Rules` + test headless.
- Systèmes ordonnés par `SystemSet` explicites : `Input → Script(pre) → Mechanics → Physics → Collisions → Script(post) → Cleanup`.
- Physique maison simple : AABB contre tilemap (grille) + AABB entre entités. Pas de moteur physique externe (déterminisme, contrôle).

## Scripting (`mw_script`)
- Moteur **Rhai** créé par niveau, sandbox : `set_max_operations`, `set_max_call_levels`, pas de module fichier/réseau.
- Exécuté **uniquement côté serveur** (et en solo dans le process local). Les clients ne voient que les effets via snapshots.
- Hooks (fonctions optionnelles définies par le script) : `on_level_start()`, `on_tick(dt)`,
  `on_player_spawn(p)`, `on_player_input(p, input)`, `on_collision(a, b)`, `on_damage(target, src, amount)`,
  `on_death(e)`, `on_trigger(name, e)`, `on_entity_tick(kind, e)` pour les entités custom.
- API (voir `docs/SCRIPTING_API.md`) : lire/écrire `rules`, requêter/modifier joueurs et entités
  (position, vélocité, PV, tags, variables custom), spawn/despawn, timers, état global persistant
  du niveau, événements, messages HUD.
- Les scripts ne mutent pas l'ECS directement : ils produisent des **commandes** appliquées par un système Rust
  (frontière claire, testable, déterministe).

## Réseau (`mw_net`)
- UDP via `renet` (canaux fiables / non fiables). `PROTOCOL_VERSION` vérifié au handshake.
- Client → serveur : inputs des derniers ticks (redondance contre la perte).
- Serveur → client : snapshots d'état (non fiable, ~20-30 Hz), événements (fiable), niveau + scripts à la connexion (fiable, chunké).
- Client : prédiction du joueur local (même code `mw_sim`) + réconciliation ; interpolation des entités distantes.
- Découverte LAN : broadcast UDP sur un port dédié. Voir `docs/NETCODE.md`.

## Rendu (`mw_render`)
- Lit l'état de la sim, n'écrit jamais dedans. Sprites animés (frames PNG, voir `assets/manifest.json`),
  tilemap, caméra qui suit, parallax, plantes animées décoratives, HUD.

## Éditeur (`mw_editor`) et MCP (`mosswake-mcp`)
- Les deux manipulent un `mw_level::Level` et passent par la **même validation**.
- Le MCP est un binaire stdio indépendant de Bevy pour l'édition ; la simulation headless (`simulate_level`)
  réutilise `mw_sim`.

## Unités
- 1 tile = **32 unités monde** (confirmé par #2 : l'art du tileset est à 512 px/tile, le rendu met à l'échelle). Y vers le haut. Positions en `f32` dans la sim.
