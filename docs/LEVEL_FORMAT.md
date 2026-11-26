# Format de niveau (brouillon — à finaliser par l'issue « mw_level schéma v1 »)

```
levels/<id>/
  level.json
  scripts/main.rhai      # optionnel, plusieurs fichiers possibles
```

Esquisse de `level.json` :
```json
{
  "schema_version": 1,
  "id": "forest_01",
  "name": "Forêt moussue",
  "size": { "w": 120, "h": 40 },
  "tile_size": 32,
  "layers": [
    { "name": "background", "kind": "decor", "tiles": "..." },
    { "name": "ground", "kind": "solid", "tiles": "..." }
  ],
  "spawns": [ { "x": 3, "y": 5 } ],
  "entities": [ { "kind": "slime_green", "x": 20, "y": 5, "props": {} } ],
  "rules": { "gravity": -1800.0, "mechanics": ["jump", "dash", "wall_jump"] },
  "scripts": ["scripts/main.rhai"]
}
```
Contraintes : JSON lisible et éditable par un LLM ; toute évolution incompatible bump `schema_version` avec migration.
