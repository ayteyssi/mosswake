Closes #

## Résumé
<!-- Ce qui a été fait, choix de conception -->

## Comment tester
<!-- Commandes, tests ajoutés, scénario manuel -->

## Checklist
- [ ] Les critères d'acceptation de l'issue sont remplis
- [ ] `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` passent
- [ ] Pas de logique gameplay hors `mw_sim`/`mw_script`, pas de constante gameplay en dur (→ `Rules`)
- [ ] Docs (`docs/*`, `CLAUDE.md`) mises à jour si nécessaire
- [ ] `schema_version` / `PROTOCOL_VERSION` bumpés si format/protocole modifiés
