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
  Implémentation : `Rules::with_overrides` sérialise les règles en objet JSON, y applique les clés
  du niveau puis les relit ; clé inconnue ou mauvais type = erreur au chargement. **Ajouter un
  paramètre = ajouter un champ avec sa valeur par défaut**, il devient surchargeable sans autre code.
- **Mécaniques = plugins activables** : chaque plugin de mécanique déclare son identifiant avec
  `app.register_mechanic("dash")` (`movement`, `jump`, `double_jump`, `dash`, `wall_jump`, `melee`, …)
  et ses systèmes tournent sous `.run_if(mechanic_active("dash"))` dans `SimSet::Mechanics` ;
  le niveau liste celles qui sont actives (`rules.mechanics`, ids inconnus refusés au chargement).
  Ajouter une mécanique = nouveau module + `register_mechanic` + paramètres dans `Rules` + test headless.
- **Joueurs** (`mw_sim::player`) : `spawn_player(world, PlayerId)` crée le corps (taille dans `Rules`)
  sur le spawn `id % nb_spawns`. `SimSet::Input` copie `TickInputs` dans `PlayerControl` (input du
  tick + précédent, pour détecter les fronts). Mécaniques `movement` (accélération/freinage sol et air)
  et `jump` (hauteur variable, coyote time et jump buffer **comptés en ticks**) actives par défaut.
  `PlayerState { Idle, Run, Jump, Fall }` + `Facing` sont mis à jour après les contacts, pour l'animation.
- Systèmes ordonnés par `SimSet` (chaînés dans `FixedUpdate`) : `Input → ScriptPre → Mechanics → Physics → Collisions → ScriptPost → Cleanup`.
- **Temps** : les systèmes n'utilisent jamais l'horloge ; ils avancent de `TICK_DT = 1/60 s` par tick.
  `SimHarness::new(&level)` + `step(n, &inputs)` exécute la sim tick par tick pour les tests et outils.
- **Identité** : `SimId` (alloué dans l'ordre de spawn) identifie une entité de façon identique sur
  toutes les machines ; `Entity` de Bevy ne doit pas sortir de la sim.

### Physique (`mw_sim::physics`, `mw_sim::grid`)
- Maison, sans moteur externe (déterminisme, contrôle). Unités monde (1 tile = `tile_size` = 32), Y vers le haut.
- `Position` = **pieds** (milieu du bas de la boîte), comme les positions du format de niveau ; `Body { size, gravity_scale }`, `Velocity` en unités/s.
- `Physics` : gravité (`Rules::gravity`, plafonnée à `max_fall_speed` dans son sens ; les corps à
  `gravity_scale = 0` ne sont ni accélérés ni plafonnés), puis déplacement **axe par axe (X puis Y)** :
  chaque colonne/ligne de tiles traversée est testée, le corps s'arrête pile contre la première tile
  solide → aucun tunneling quelle que soit la vitesse ; la vitesse est annulée sur l'axe bloqué.
- `Collisions` : drapeaux `Contacts { grounded, on_ceiling, on_wall_left, on_wall_right }` par sondes de
  0,01 unité. `grounded` suit le sens de la gravité (gravité inversée → « sol » au-dessus).
  Les collisions entre entités (dégâts, triggers) s'ajouteront dans ce set.
- Hors de la grille : colonnes à gauche/droite **solides** (murs invisibles), lignes en dessous/au-dessus
  **vides** (on peut tomber hors du niveau ; la mort/respawn relève de #9).

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
  tilemap, caméra qui suit, parallax, plantes animées décoratives, HUD (#11).
- `ClientRenderPlugin::new(assets_root, joueur_suivi)` charge le manifest et l'atlas ; il ajoute ses
  propres composants (sprite, transform…) aux entités de la sim, sans toucher aux composants de gameplay.
- **Décisions pures dans `mw_render::layout`** (testées sans GPU) : pièce d'atlas par cellule
  (autotiling, voisins = même id de légende, hors niveau = plein), emplacements des plantes, ancrage
  depuis le pivot, bornes de caméra. Un id de légende inconnu de l'atlas est dessiné en placeholder magenta.
- **Textures réduites au chargement** (`textures.rs`) à ~64 px/tile avant l'envoi au GPU (÷16 en VRAM,
  pas de scintillement sans mipmaps) ; seuls les sprites utilisés sont chargés.
- **Interpolation** : `PrevPosition` mémorisée en `FixedPreUpdate`, affichage à
  `lerp(prev, position, overstep_fraction)` : mouvement fluide à toute fréquence d'écran.
- Caméra orthographique de 15 tiles de haut, suivi exponentiel (plus souple en vertical), bornée au niveau.
  Parallax : `pos = base + caméra × facteur` (0 = fixe dans le monde, 1 = fixe à l'écran), silhouettes
  translucides pour l'effet de profondeur.
- Entrées (`LocalInputPlugin`) : clavier + manette → `PlayerInput`, appuis brefs entre deux ticks
  mémorisés jusqu'au tick suivant. Clavier : A/D ou ←/→, Espace/K/Z saut, Maj/L/X dash, J/C attaque,
  E/↑ interagir. Manette : stick gauche/croix, Sud saut, Est/gâchette droite dash, Ouest attaque.
- En attendant #8, les entités placées par le niveau (slimes) sont dessinées à leur position de départ.

## Éditeur (`mw_editor`) et MCP (`mosswake-mcp`)
- Les deux manipulent un `mw_level::Level` et passent par la **même validation**.
- Le MCP est un binaire stdio indépendant de Bevy pour l'édition ; la simulation headless (`simulate_level`)
  réutilise `mw_sim`.

## Unités
- 1 tile = **32 unités monde** (confirmé par #2 : l'art du tileset est à 512 px/tile, le rendu met à l'échelle). Y vers le haut. Positions en `f32` dans la sim.
