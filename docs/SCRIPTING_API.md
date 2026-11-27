# API de scripting Rhai

Chaque fonction exposée à Rhai **doit** être listée ici avec signature, description, exemple, et avoir un test.
Langage : [Rhai](https://rhai.rs/book/) (syntaxe proche de Rust/JS : `let`, `fn`, `if`, `while`, `loop`,
tableaux `[..]`, objets `#{..}`, chaînes `"..."` et `` `${interpolation}` ``).

État : tout ce qui est décrit ici est implémenté. Le moteur et les commandes sont dans `mw_script`
(**#18**, **#19**). Le branchement dans la simulation (`mw_sim::scripting`, **#19**) appelle les hooks
au bon moment de chaque tick et applique les commandes. Les tests cités sont dans
`crates/mw_script/tests/` et `crates/mw_sim/tests/scripting.rs`.

## Fichiers et exécution
- Un niveau liste ses scripts dans `level.json` → `scripts` (`scripts/**/*.rhai`), exécutés **dans cet ordre**.
- **Chaque fichier est indépendant** : ses fonctions ne sont pas visibles depuis les autres fichiers.
  Tous partagent l'objet `state`.
- Le **code de premier niveau** (hors fonctions) s'exécute **une fois au chargement**, avant `on_level_start`.
  C'est l'endroit où initialiser `state`.
- Les scripts tournent **uniquement sur le serveur** (ou dans le process local en solo). Ils ne
  modifient pas le jeu directement : ils émettent des **commandes** que la simulation valide et applique.
- **Unités** : positions en **tiles** (comme `level.json`, origine en bas à gauche, Y vers le haut,
  position = milieu du bas du corps), vitesses en **tiles par seconde**, durées en **secondes**
  (converties en ticks de 1/60 s). `rules` reste en unités monde, comme dans `level.json`.

### Déroulement d'un tick
1. **ScriptPre** (avant les mécaniques) : au premier tick, `on_level_start()` ; puis
   `on_player_spawn(p)` pour chaque nouveau joueur (ordre des ids) ; puis `on_tick(dt)` ; puis les
   timers échus.
2. Mécaniques, physique, combat. Chaque coup qui va porter passe par `on_damage`.
3. **ScriptPost** (après les collisions) : `on_death`, `on_collision`, puis `on_trigger`.

Chaque hook voit l'état du monde **au moment où il est appelé**. Ses commandes sont appliquées dès
la fin de l'appel, donc le hook suivant les voit. À l'intérieur d'un même appel, un script relit ses
propres écritures (`p.x = 3; p.x` vaut 3).

## Hooks
Fonctions **optionnelles** : un script ne définit que celles dont il a besoin. Un hook est reconnu par
son nom **et** son nombre de paramètres. Si plusieurs scripts définissent un hook, ils sont tous
appelés, dans l'ordre de `scripts`.

| Hook | Quand |
|---|---|
| `on_level_start()` | Une fois, au premier tick du niveau |
| `on_tick(dt)` | Chaque tick de simulation (`dt` = 1/60 s, flottant) |
| `on_player_spawn(player)` | Une fois par joueur apparu, au tick suivant son apparition |
| `on_collision(a, b)` | Quand deux corps **commencent** à se toucher (une seule fois par contact ; `a.id < b.id`) |
| `on_damage(target, source, amount)` | Avant qu'un coup ne porte. **Retourne** le montant final (entier) |
| `on_death(target)` · `on_death(target, source)` | Mort d'une entité (PV à 0) |
| `on_trigger(name, entity)` | Un corps **entre** dans une zone `trigger` du niveau |

- `source` vaut `()` quand il n'y a pas d'auteur (dégâts de script, d'environnement).
- **`on_damage` est un filtre** : chaque script qui le définit reçoit le montant laissé par le
  précédent. Retourner autre chose qu'un entier (ou rien) laisse le montant inchangé, et **0 annule**
  le coup (pas de recul ni d'invincibilité). Il n'est appelé que pour un coup qui porterait :
  jamais sur une cible déjà morte ou invincible. Un `damage()` émis pendant `on_damage` est appliqué
  au tick suivant, ce qui exclut toute récursion.
- `on_collision` concerne les corps solides (joueurs, entités), pas les zones trigger.

Tests : `on_player_spawn_runs_once_per_player`, `on_collision_fires_when_contact_starts`,
`on_damage_changes_hits_and_on_death_sees_the_kill`, `trigger_zones_from_the_level_call_on_trigger_on_entry`,
`filter_hook_chains_return_values`.

## État persistant : `state`
Objet (`#{}`) global, **conservé entre tous les appels** et partagé par tous les scripts du niveau,
accessible aussi **à l'intérieur des fonctions**. Le nom `state` est réservé.
```rhai
state.lava_y = 0.0;              // premier niveau : initialisation au chargement

fn on_tick(dt) {
    state.lava_y += 20.0 * dt;   // persiste d'un tick à l'autre
}
```
Test : `hooks_are_called_and_state_persists`, `scripts_share_state_but_not_functions`.

## Fonctions
### `hud(text)` · `hud(text, seconds)`
Affiche `text` à tous les joueurs pendant `seconds` (défaut **3,0**). Émet `ScriptCommand::Hud`.
```rhai
fn on_level_start() { hud("La lave monte !", 5.0); }
```
Les messages actifs sont dans la ressource `HudMessages` (texte, ticks restants) ; leur affichage
côté client relève de #11.
Tests : `hooks_are_called_and_state_persists`, `commands_set_rule_and_print`,
`hud_messages_last_their_duration`.

### `rules`
Objet des règles de jeu, accessible partout comme `state`. Il contient toutes les clés de `Rules`
(voir `docs/LEVEL_FORMAT.md` → *Règles*), y compris `mechanics`. Une **modification** devient
`ScriptCommand::SetRule` à la fin de l'appel, et seules les clés réellement changées en produisent
une. La simulation la vérifie comme une surcharge de `level.json` : une clé inconnue ou une valeur
invalide (mauvais type, `run_speed` négatif…) est **refusée** et notée dans `ScriptLog.rejected`, et
la règle garde sa valeur. Si l'appel échoue, ses modifications sont annulées.
```rhai
fn on_level_start() {
    rules.gravity = -rules.gravity;    // gravité inversée
    rules.jump_height += 32.0;
}
```
Tests : `rules_edits_become_set_rule_commands`, `scripts_read_and_write_every_rule_and_toggle_mechanics`,
`a_failing_call_rolls_back_its_rule_edits_and_timers`.

### `set_rule(key, value)`
Équivalent de `rules[key] = value`, en une commande explicite. `value` : nombre, booléen, chaîne,
tableau ou objet de ces types ; une valeur non convertible (fonction, NaN…) est une erreur
d'exécution du script.
```rhai
set_rule("mechanics", ["movement"]);    // plus de saut dans ce niveau
```
Tests : `commands_set_rule_and_print`, `scripts_read_and_write_every_rule_and_toggle_mechanics`.

### `enable_mechanic(id)` · `disable_mechanic(id)` · `mechanic_active(id)`
Ajoute ou retire `id` de `rules.mechanics` (ids : `movement`, `jump`, `dash`, `melee`…). Activer une
mécanique déjà active ne change rien. `mechanic_active` lit l'état courant, modifications de l'appel
comprises. Un id inconnu est refusé par la simulation.
```rhai
fn on_trigger(name, e) { if name == "dash_shrine" { enable_mechanic("dash"); } }
```
Tests : `reading_rules_and_toggling_mechanics`, `scripts_read_and_write_every_rule_and_toggle_mechanics`.

### Entités : le type `Entity`
Valeur passée aux hooks et renvoyée par `players()`, `entities()`, `entity(id)` et
`entities_with_tag(tag)`. Les lectures donnent l'état au début de l'appel, plus les écritures du
script lui-même. Les écritures deviennent des commandes appliquées à la fin de l'appel.

| Propriété / méthode | Description |
|---|---|
| `e.id` | Id de simulation (entier, stable ; le même que dans les snapshots) |
| `e.kind` | `"player"`, le `kind` de l'entité (`"slime_green"`, un kind de script…), `"trigger"` ou `"body"` |
| `e.is_player` · `e.player` | Joueur ? · numéro de slot (`()` sinon) |
| `e.x`, `e.y` *(lecture/écriture)* | Position en tiles |
| `e.vx`, `e.vy` *(lecture/écriture)* | Vitesse en tiles/s |
| `e.hp` · `e.max_hp` · `e.alive` | PV courants et maximum (`()` si le corps ne prend pas de dégâts) · vivant |
| `e.teleport(x, y)` | Place le corps en `(x, y)` et annule sa vitesse |
| `e.damage(n)` · `e.damage(n, cause)` | Dégâts **mis en file** : appliqués au plus tôt au combat du tick, via `on_damage` et l'invincibilité |
| `e.heal(n)` | Rend `n` PV, sans dépasser le maximum |
| `e.add_tag(t)` · `e.remove_tag(t)` · `e.has_tag(t)` | Étiquettes libres |
| `e.set(key, value)` · `e.get(key)` | Variables custom stockées sur le corps (`()` si absente) |
| `e.despawn()` | Retire le corps |
| `a == b` | Même id |

Les nombres écrits (`x`, `vx`, `teleport`, `spawn_entity`…) doivent être **finis** : NaN ou
l'infini donne une erreur d'exécution du script. Les vitesses énormes sont arrêtées par les murs
invisibles du niveau.

Un id disparu (corps despawné entre-temps) ne fait pas d'erreur : la commande est refusée et notée
dans `ScriptLog.rejected`.
```rhai
fn on_tick(dt) {
    for p in players() {
        if p.y < state.lava_y { p.damage(999, "lava"); }
    }
}
```
Tests : `entities_read_the_view_and_write_commands`, `entity_api_moves_damages_heals_tags_and_spawns`,
`despawn_and_unknown_ids_are_handled`, `hostile_values_cannot_corrupt_or_stall_the_simulation`.

### `players()` · `entities()` · `entity(id)` · `entities_with_tag(tag)`
Tableaux d'`Entity` triés par id (`entity` renvoie `()` si l'id n'existe pas). `entities()` contient
aussi les joueurs et les zones trigger.
Tests : `entities_read_the_view_and_write_commands`, `despawn_and_unknown_ids_are_handled`.

### `spawn_entity(kind, x, y)` · `spawn_entity(kind, x, y, props)`
Crée une entité de type `kind` (chaîne libre), pieds en `(x, y)` tiles. `props` : `w`, `h` (taille
en tiles, défaut 1, bornée à `[1/16, taille du niveau]`), `gravity` (échelle, défaut 1 ; 0 = flotte), `hp` (la rend vulnérable), `name`
(pour `kind = "trigger"`). Toutes les `props` deviennent aussi ses variables (`e.get`). Son id
n'existe qu'à l'application de la commande : on la retrouve ensuite avec `entities()`. (`spawn` est un
mot réservé de Rhai, d'où le nom.)
```rhai
every(2.0, || spawn_entity("rock", 10, 18, #{ hp: 1 }));
```
Tests : `entity_api_moves_damages_heals_tags_and_spawns`, `scripted_runs_are_deterministic`.

### `after(seconds, fn)` · `every(seconds, fn)` · `cancel(id)`
Planifie `fn` (une closure `|| …`) **une fois** après `seconds`, ou **toutes les** `seconds` (au
minimum 1 tick). La durée est arrondie au tick, ce qui rend les timers déterministes. Les deux
renvoient un id à passer à `cancel` ; un timer `every` peut s'annuler lui-même. Les timers échus
s'exécutent après `on_tick`, par échéance puis par ordre de création. Un timer appartient au script
qui l'a créé et voit `state`, `rules` et les entités comme un hook.
```rhai
fn on_level_start() {
    after(3.0, || hud("Go !"));
    state.wave = every(10.0, || spawn_entity("slime_green", 30, 2));
}
```
Tests : `timers_fire_on_their_tick_repeat_and_cancel`, `a_repeating_timer_can_cancel_itself`,
`timers_run_on_exact_ticks`.

### `print(value)` · `debug(value)`
Écrit dans le journal du serveur (`[script] …`) et la console de debug (`ScriptCommand::Log`).
`debug` préfixe le fichier et la position. Côté sim, tout arrive dans la ressource `ScriptLog`
(`lines`, `errors`, `rejected`, 256 entrées max chacune), que le serveur vide et affiche à chaque tick.
Test : `commands_set_rule_and_print`.

### Langage standard
Disponibles : arithmétique, logique, chaînes, tableaux, objets, maths (`sin`, `sqrt`, `floor`…),
`parse_int`/`parse_float`, `type_of`… (paquet standard de Rhai).

## Bac à sable
- **Absents par construction** : accès fichiers et réseau, `import` (feature `no_module`), horloge
  `timestamp()` (feature `no_time`, non déterministe), `eval`.
- **Limites par appel de hook** (`mw_script::Limits`) : 100 000 opérations, 32 niveaux d'appel,
  profondeur d'expression 64/32, chaînes ≤ 4096 octets, tableaux ≤ 4096 éléments, objets ≤ 1024
  propriétés, 512 variables. Un script qui dépasse est **interrompu** (`LimitExceeded`) ; les autres
  scripts et les ticks suivants continuent.
- Déterministe : mêmes scripts + mêmes appels → mêmes commandes et même `state`.

Tests : `infinite_loop_is_interrupted_and_the_host_survives`, `sandbox_limits_recursion_and_data_size`,
`sandbox_has_no_eval_import_or_clock`, `hosts_are_deterministic`.

## Erreurs
Jamais de panic. Chaque erreur est un `ScriptError` sérialisable : `script` (chemin), `context`
(`<compile>`, `<load>` ou nom du hook), `kind` (`syntax`, `runtime`, `limit_exceeded`), `line`,
`column`, `message`. Exemple affiché : `scripts/main.rhai:3:3: Runtime: Function not found: explode (i64)`.
- Erreur de syntaxe ou au chargement : le niveau ne démarre pas (`SimLoadError::Scripts`, toutes
  les erreurs sont listées).
- Erreur dans un hook ou un timer : ce script est ignoré **pour cet appel** (ses commandes, ses
  modifications de `rules` et ses nouveaux timers sont annulés), les autres scripts s'exécutent.
  Un timer `every` qui échoue n'est pas replanifié.
- Test : `script_errors_are_reported_without_stopping_the_game`.
- Une limite atteinte n'a pas de position propre : on indique l'appel fait depuis le hook quand il existe.

## Déterminisme et réseau
- Entrées ordonnées (ids croissants), timers en ticks, pas d'horloge ni d'aléatoire : deux parties
  avec les mêmes inputs donnent le même monde au bit près (test `scripted_runs_are_deterministic`).
- Les scripts ne tournent **que là où la simulation fait autorité** (serveur, solo). La copie de
  prédiction d'un client n'en exécute pas : elle se recale sur les snapshots du serveur.

## Exemple complet
```rhai
state.lava_y = 0.0;

fn on_level_start() {
    rules.gravity = -rules.gravity;          // gravité inversée
    enable_mechanic("dash");
    hud("La lave monte !", 5.0);
    every(5.0, || spawn_entity("rock", 12, 18, #{ hp: 1 }));
}

fn on_tick(dt) {
    state.lava_y += 0.5 * dt;                // tiles par seconde
    for p in players() {
        if p.y < state.lava_y { p.damage(999, "lava"); }
    }
}

fn on_damage(target, source, amount) {
    if target.has_tag("shield") { 0 } else { amount }
}

fn on_trigger(name, e) {
    if name == "shrine" && e.is_player { e.add_tag("shield"); e.heal(99); }
}
```
