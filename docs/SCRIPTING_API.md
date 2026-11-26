# API de scripting Rhai

Chaque fonction exposée à Rhai **doit** être listée ici avec signature, description, exemple, et avoir un test.
Langage : [Rhai](https://rhai.rs/book/) (syntaxe proche de Rust/JS : `let`, `fn`, `if`, `while`, `loop`,
tableaux `[..]`, objets `#{..}`, chaînes `"..."` et `` `${interpolation}` ``).

État : **#18** (moteur, hooks de base, `state`, `hud`, `set_rule`, `print`) est implémenté dans `mw_script`.
Le branchement dans la simulation (application des commandes, accès aux joueurs et entités, timers,
autres hooks) arrive avec **#19**. Les sections marquées *(#19)* décrivent l'API visée.

## Fichiers et exécution
- Un niveau liste ses scripts dans `level.json` → `scripts` (`scripts/**/*.rhai`), exécutés **dans cet ordre**.
- **Chaque fichier est indépendant** : ses fonctions ne sont pas visibles depuis les autres fichiers.
  Tous partagent l'objet `state`.
- Le **code de premier niveau** (hors fonctions) s'exécute **une fois au chargement**, avant `on_level_start`.
  C'est l'endroit où initialiser `state`.
- Les scripts tournent **uniquement sur le serveur** (ou dans le process local en solo). Ils ne
  modifient pas le jeu directement : ils émettent des **commandes** que la simulation valide et applique.

## Hooks
Fonctions **optionnelles** : un script ne définit que celles dont il a besoin. Un hook est reconnu par
son nom **et** son nombre de paramètres. Si plusieurs scripts définissent un hook, ils sont tous
appelés, dans l'ordre de `scripts`.

| Hook | Quand | État |
|---|---|---|
| `on_level_start()` | Une fois, au démarrage du niveau | #18 |
| `on_tick(dt)` | Chaque tick de simulation (`dt` = 1/60 s, flottant) | #18 |
| `on_player_spawn(player)` | Apparition d'un joueur | *(#19)* |
| `on_collision(a, b)` | Contact entre deux entités | *(#19)* |
| `on_damage(target, source, amount)` | Avant application des dégâts (retourne le montant final) | *(#19)* |
| `on_death(entity)` | Mort d'une entité | *(#19)* |
| `on_trigger(name, entity)` | Entrée dans une zone trigger | *(#19)* |

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
Tests : `hooks_are_called_and_state_persists`, `commands_set_rule_and_print`.

### `set_rule(key, value)`
Modifie une règle de jeu (`key` = une clé de `Rules`, voir `docs/LEVEL_FORMAT.md` → *Règles*). `value` :
nombre, booléen, chaîne, tableau ou objet de ces types. Émet `ScriptCommand::SetRule` ; la simulation
la vérifie comme une surcharge de `level.json` (clé inconnue ou mauvais type → erreur rapportée).
Une valeur non convertible (fonction, NaN…) est une erreur d'exécution du script.
```rhai
fn on_level_start() {
    set_rule("gravity", 1800);              // gravité inversée
    set_rule("mechanics", ["movement"]);    // plus de saut dans ce niveau
}
```
Tests : `commands_set_rule_and_print`.

### `print(value)` · `debug(value)`
Écrit dans le journal du serveur et la console de debug (`ScriptCommand::Log`). `debug` préfixe le
fichier et la position.
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
- Erreur de syntaxe ou au chargement : le niveau ne démarre pas (toutes les erreurs sont listées).
- Erreur dans un hook : ce script est ignoré **pour cet appel** (ses commandes de l'appel sont
  annulées), les autres scripts s'exécutent.
- Une limite atteinte n'a pas de position propre : on indique l'appel fait depuis le hook quand il existe.

## Exemple visé (avec #19)
```rhai
fn on_level_start() {
    rules.gravity = -rules.gravity;     // gravité inversée      (aujourd'hui : set_rule)
    rules.max_jumps = 3;
}

fn on_tick(dt) {
    state.lava_y += 20.0 * dt;          // la lave monte
    for p in players() {
        if p.y < state.lava_y { p.damage(999, "lava"); }
    }
}
```
