# Workflow des agents

Chaque tâche est une **issue GitHub**. Un agent = une issue à la fois. Les PR sont validées par
un agent reviewer ; **ne merge jamais toi-même**.

## Labels
- `status:ready` — prête à être prise (dépendances fermées).
- `status:blocked` — dépend d'issues ouvertes (`Blocked by #N` dans la description).
- `status:in-progress` — claim par un agent.
- `status:review` — PR ouverte, en attente de review.
- `area:*`, `priority:p0|p1|p2`, `size:S|M|L`.

## 1. Choisir une issue
```sh
gh issue list --label status:ready --search "no:assignee" --json number,title,labels,milestone
```
Prends la plus prioritaire (`priority:p0` d'abord, puis milestone la plus basse). Vérifie que
toutes les issues listées dans `Blocked by` sont **fermées**.

## 2. La claim (immédiatement, avant de coder)
```sh
gh issue edit <N> --add-assignee @me --remove-label status:ready --add-label status:in-progress
gh issue comment <N> --body "🤖 Claimed by agent <ton-identifiant>. Branch: issue-<N>-<slug>"
```
Puis relis l'issue : si un autre agent a commenté une claim avant toi, abandonne-la
(retire ton assignation/label) et choisis-en une autre.

## 3. Branche
```sh
git checkout main && git pull --ff-only
git checkout -b issue-<N>-<slug-court>
```

## 4. Implémenter
- Respecte `CLAUDE.md` et les critères d'acceptation de l'issue.
- Commits petits et clairs (`feat(sim): coyote time (#N)`).
- Avant de pousser : `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`.
- Rebase sur `main` si besoin (`git fetch && git rebase origin/main`).

## 5. Pull request
```sh
git push -u origin HEAD
gh pr create --fill --base main --label status:review   # puis complète le template
gh issue edit <N> --remove-label status:in-progress --add-label status:review
```
La description **doit** contenir `Closes #<N>`, ce qui a été fait, comment tester, et la checklist cochée.

## 6. Review
Le reviewer vérifie la CI, lance les tests, confronte le diff aux critères d'acceptation, puis :
- **approve + squash-merge** → l'issue se ferme ;
- ou **request changes** → corrige sur la même branche, pousse, et commente « ready for re-review ».

## 7. Après le merge
Si ta PR débloque des issues (`Blocked by #<N>`) et que toutes leurs dépendances sont fermées,
passe-les en `status:ready` :
```sh
gh issue edit <M> --remove-label status:blocked --add-label status:ready
```

## Abandon
Si tu ne peux pas finir : commente l'état d'avancement, pousse la branche, retire ton
assignation et remets `status:ready`.

## Nouvelles tâches
Besoin découvert hors périmètre → nouvelle issue avec le template « Task » (contexte, objectif,
critères d'acceptation, dépendances, labels).
