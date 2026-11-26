# Netcode

Implémenté dans `mw_net` (sans dépendance à `mw_sim`). Les parties non encore implémentées
sont signalées par leur issue.

## Vue d'ensemble
- Transport : UDP via `renet` (canaux, fiabilité) sur `renet_netcode` (connexions, timeouts).
  Port de jeu par défaut **47800** (`DEFAULT_PORT`), découverte LAN en broadcast sur **47801**
  (`DISCOVERY_PORT`, #15).
- Mode **non sécurisé** de netcode (pas de jeton signé ni de clé) : jeu en réseau local de confiance.
- Serveur autoritaire, tick 60 Hz, snapshots ~30 Hz (#13), prédiction client + réconciliation,
  interpolation ~100 ms (#14).
- Sérialisation : `postcard` (varints compacts), via `protocol::encode` / `decode`
  (les octets en trop sont refusés).

## Canaux
Mêmes identifiants dans les deux sens ; chaque message connaît son canal (`message.channel()`).

| Canal | Garantie | Client → serveur | Serveur → client |
|---|---|---|---|
| `Unreliable` (0) | aucune, le suivant remplace | `Inputs` | `Snapshot` |
| `Reliable` (1) | fiable et ordonné | (contrôle, à venir) | réponse de handshake, `Roster`, `PlayerJoined`, `PlayerLeft` |

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
- Inputs : `Inputs { first_tick, inputs }` porte les derniers ticks (redondance contre la perte).

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
