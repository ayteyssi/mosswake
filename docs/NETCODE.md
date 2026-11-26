# Netcode

Transport et protocole dans `mw_net` (sans dépendance à `mw_sim`) ; le serveur de jeu qui relie
la simulation au réseau est dans `bins/mosswake` (voir « Serveur de jeu »). Les parties non
encore implémentées sont signalées par leur issue.

## Vue d'ensemble
- Transport : UDP via `renet` (canaux, fiabilité) sur `renet_netcode` (connexions, timeouts).
  Port de jeu par défaut **47800** (`DEFAULT_PORT`), découverte LAN en broadcast sur **47801**
  (`DISCOVERY_PORT`, #15).
- Mode **non sécurisé** de netcode (pas de jeton signé ni de clé) : jeu en réseau local de confiance.
- Serveur autoritaire, tick 60 Hz, snapshots 30 Hz, prédiction client + réconciliation,
  interpolation ~100 ms (#14).
- Sérialisation : `postcard` (varints compacts), via `protocol::encode` / `decode`
  (les octets en trop sont refusés).

## Canaux
Mêmes identifiants dans les deux sens ; chaque message connaît son canal (`message.channel()`).

| Canal | Garantie | Client → serveur | Serveur → client |
|---|---|---|---|
| `Unreliable` (0) | aucune, le suivant remplace | `Inputs` | `Snapshot` |
| `Reliable` (1) | fiable et ordonné | `LevelRequest`, `LevelReady` | réponse de handshake, `Roster`, `PlayerJoined`, `PlayerLeft`, `LevelOffer`, `LevelChunk` |

## Connexion et versions
Problème : pour refuser proprement un client d'une autre version, il faut lui envoyer un message
qu'il sait lire, alors que le format des messages change justement entre versions. Solution :
une petite couche **gelée** (`mw_net::handshake`), lisible par toutes les versions.

1. Le client se connecte avec le `protocol_id` netcode **constant** `NETCODE_PROTOCOL_ID`
   (« MOSSWAKE »). Il ne contient pas de version : sinon netcode ignorerait silencieusement le client.
2. Les `user_data` de netcode (256 octets) portent la demande gelée : `[0..4]` `PROTOCOL_VERSION`
   (u32 LE), `[4]` longueur du nom, `[5..]` nom UTF-8 (≤ 24 octets, sans caractère de contrôle).
3. Le serveur répond, en **premier message** du canal fiable, par un `HandshakeReply` gelé :
   `Accepted { player }`, ou `Rejected(reason)` avec `VersionMismatch { server, client }`,
   `MalformedRequest`, `InvalidName` ou `ServerFull`. Un client refusé reste connecté 500 ms pour
   recevoir la raison, puis il est coupé ; le client se déconnecte de lui-même en la lisant.
4. Une fois accepté, le client reçoit `Roster` (joueurs présents, triés par id), et les autres
   joueurs reçoivent `PlayerJoined`.

Les encodages gelés sont vérifiés octet par octet par des tests. Règle : ne jamais modifier
ni réordonner, seulement **ajouter** des variantes en fin d'enum.

Tout le reste (`protocol::ClientMessage`, `ServerMessage`, …) est versionné :
**toute modification d'un message incrémente `PROTOCOL_VERSION`**.

## Session
- `PlayerId` : plus petit slot libre (`0..max_players`). Le netcode accepte quelques connexions de
  plus que `max_players` pour pouvoir répondre `ServerFull` explicitement.
- Départ : `PlayerLeft { player, reason }` est diffusé aux autres, avec `reason` = `Left`
  (déconnexion volontaire), `TimedOut` (silence ≥ `TIMEOUT`) ou `Kicked` (exclusion, ou message
  indécodable = erreur de protocole).
- `TIMEOUT` = **15 s**, imposé par renetcode en mode non sécurisé.
- Inputs : `Inputs { first_tick, inputs }` porte les derniers ticks (redondance contre la perte),
  voir « Application des inputs ».

## Transfert du niveau (#16)
Les clients n'ont pas forcément le niveau (niveaux créés par IA) : le serveur l'envoie.
1. **Paquet** : `LevelBundle::to_package()` (`mw_level`) = JSON canonique `{level, scripts}`. Un même
   niveau donne les mêmes octets sur tout serveur, donc le même hash. Au déballage, les chemins
   de scripts sont revalidés (un serveur ne peut pas faire écrire hors de `scripts/`) et le schéma
   est migré.
2. **Offre** (fiable) : à chaque connexion, et à chaque changement de niveau pour tous,
   `LevelOffer { transfer, hash (SHA-256), size, chunks }`. `transfer` augmente à chaque niveau :
   les messages qui portent un ancien numéro sont ignorés.
3. Le client (`mosswake::level_sync::LevelSync`) regarde son **cache** (`LevelCache`, optionnel,
   `<dir>/<hash>.mwlevel`, relu et revérifié) :
   - présent : il charge et répond `LevelReady` (aucun téléchargement) ;
   - absent : `LevelRequest`, le serveur envoie `LevelChunk { transfer, index, data }` (16 Kio
     chacun, fiables). `mw_net::Assembler` reconstitue dans n'importe quel ordre, refuse les offres
     incohérentes ou > **4 Mio** et vérifie le **hash**. Le client met en cache, charge, valide le
     niveau, puis répond `LevelReady`.
4. Le serveur ne fait apparaître le joueur **qu'à réception de `LevelReady`**. Avant cela, le
   joueur est connecté mais absent du monde, et le client ignore les snapshots.
5. `ServerSession::change_level(bundle)` : nouvelle simulation, nouvelle offre à tous ; chacun
   réapparaît en ayant chargé le nouveau niveau.

Les scripts sont transmis avec le niveau mais **jamais exécutés côté client** (ils tournent sur
le serveur). Le mode `host` passe par le même chemin : son client local télécharge le niveau en
loopback comme un joueur distant.

## Utilisation
Boucle par frame, avec le temps écoulé passé explicitement (aucune lecture d'horloge interne,
donc testable en accéléré) :
```rust
server.update(dt)?;                    // réception, handshakes, timeouts
for event in server.events() { /* PlayerConnected / PlayerDisconnected / ClientRejected */ }
for (player, msg) in server.receive() { /* ClientMessage */ }
server.broadcast(&ServerMessage::Snapshot { tick, data });
server.flush();                        // envoi des paquets

client.update(dt)?;                    // ClientState: Connecting → Connected / Rejected / Disconnected
for msg in client.receive() { /* ServerMessage */ }
client.send(&ClientMessage::Inputs { first_tick, inputs });
client.flush()?;
```
Les tests (`crates/mw_net/tests/lan.rs`) font tourner un serveur et plusieurs clients sur
`127.0.0.1` avec un temps logique ; un timeout de 15 s y prend environ une seconde réelle.

## Serveur de jeu (`bins/mosswake`)
```sh
mosswake server --headless --level <id> [--port 47800] [--max-players 4] [--levels-dir levels]
mosswake host --level <id> [--port 47800] [--name Host] [--max-players 4] [--levels-dir levels]
```
- **`ServerSession`** (`mosswake::server`) : possède la seule vraie simulation (`mw_sim`, pilotée
  tick par tick via `SimHarness`) et un `NetServer`. `tick()` fait, dans l'ordre : réception
  réseau → `spawn_player` à chaque `PlayerConnected` / `despawn_player` à chaque
  `PlayerDisconnected` → rangement des inputs reçus (un client exclu pour message indécodable
  est retiré dès ce tick) → **un** pas de simulation → snapshot si le
  tick est pair → envoi.
- **Boucle temps réel** (`main.rs`) : appelle `tick()` à 60 Hz d'horloge murale. C'est le seul
  endroit qui lit l'horloge, et seulement pour cadencer ; après un gel de plus de 250 ms
  (débogueur, mise en veille), elle reprend au lieu de rattraper des centaines de ticks.
- **Mode host** (`HostSession`, `mosswake::host`) : `ServerSession` + le client du joueur hôte
  dans le même process, pompés par la même boucle (pas de thread). Le client local passe par
  l'UDP en loopback comme un client distant : même code, aucun avantage pour l'hôte. Tant que le
  rendu (#10) n'existe pas, `mosswake host` fait tourner la session sans afficher et le joueur
  local reste immobile.
- Le niveau est chargé depuis `levels/<id>/` et validé (`LevelBundle::validate`) avant démarrage.
  Les clients distants doivent, pour l'instant, avoir le même niveau en local ; l'envoi du niveau
  à la connexion relève de #16.

### Ticks
- `Tick` = nombre de ticks simulés. Un snapshot de tick `N` est l'état **après** `N` ticks ; le
  prochain tick simulé par le serveur est alors `N`.
- Un input « pour le tick `T` » est appliqué pendant le pas qui fait passer `Tick` de `T` à `T + 1`.

### Application des inputs (`InputBuffer`)
- Le serveur range les inputs par tick. Un input pour un tick déjà simulé arrive **trop tard** et
  est ignoré ; un input plus de `MAX_INPUT_LEAD_TICKS` (120 = 2 s) en avance est ignoré (borne la
  mémoire qu'un client peut faire consommer).
- Un tick sans input reprend le **dernier input reçu** : un paquet perdu ne fait pas s'arrêter le
  joueur. Les copies redondantes sont sans effet.

### Envoi des inputs côté client (`InputSender`)
- Chaque message répète les `INPUT_REDUNDANCY` (4) derniers inputs.
- Le client vise `tick du dernier snapshot + INPUT_LEAD_TICKS` (3 ticks = 50 ms), puis +1 par
  tick. **Sans cette avance, la redondance ne sert à rien** : les copies arriveraient toujours après
  leur tick. Avec elle, jusqu'à 2 messages perdus d'affilée ne coûtent rien (testé avec 2 messages
  sur 3 perdus). Prix : 50 ms de délai sur les réactions du joueur, que la prédiction client (#14)
  masquera ; #14 remplacera aussi cette estimation par une vraie synchronisation qui tient compte
  de la latence.
- Si le client se retrouve plus de `RESYNC_AHEAD_TICKS` (30) ticks en avance sur le dernier snapshot
  (serveur figé, horloges qui dérivent), il se recale : sinon il dériverait jusqu'à ce que le serveur
  ignore tous ses inputs comme trop en avance.

### Snapshot (`mw_sim::Snapshot`)
Charge utile de `ServerMessage::Snapshot { tick, data }` : `data` = `mw_sim::Snapshot` encodé avec
`protocol::encode` (postcard). Envoyé tous les `SNAPSHOT_INTERVAL_TICKS` (2) ticks, soit 30 Hz.

| Champ | Contenu |
|---|---|
| `tick` | égal au `tick` du message |
| `bodies[]` | tous les corps, **triés par `SimId`** |
| `bodies[].id`, `position`, `velocity`, `contacts` | état physique (pieds, unités monde, y vers le haut) |
| `bodies[].player` | si c'est un joueur : `player` (`PlayerId`), `state` et `facing` (animation), `control` (inputs et minuteurs de saut, pour que le client re-simule son joueur à l'identique, #14) |

Un client trouve son propre corps avec `snapshot.player(mon_player_id)`. L'encodage du snapshot
fait **partie du protocole** : un test (`bins/mosswake/tests/server.rs`) fige ses octets, et toute
modification des types de `mw_sim::snapshot` (ou de ce qu'ils contiennent : `Contacts`,
`PlayerControl`, `PlayerState`, `Facing`) incrémente `PROTOCOL_VERSION`.
