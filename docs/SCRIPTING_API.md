# API de scripting Rhai (à compléter au fur et à mesure)

Chaque fonction exposée à Rhai **doit** être listée ici avec signature, description, exemple, et avoir un test.

## Hooks
| Hook | Quand |
|---|---|
| `on_level_start()` | Chargement du niveau |
| `on_tick(dt)` | Chaque tick de simulation |
| `on_player_spawn(player)` | Apparition d'un joueur |
| `on_collision(a, b)` | Contact entre deux entités |
| `on_damage(target, source, amount)` | Avant application des dégâts (retourne le montant final) |
| `on_trigger(name, entity)` | Entrée dans une zone trigger |

## Exemple visé
```rhai
fn on_level_start() {
    rules.gravity = -rules.gravity;     // gravité inversée
    rules.max_jumps = 3;
}

fn on_tick(dt) {
    state.lava_y += 20.0 * dt;          // la lave monte
    for p in players() {
        if p.y < state.lava_y { p.damage(999, "lava"); }
    }
}
```
