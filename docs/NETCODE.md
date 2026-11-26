# Netcode (à détailler par les issues M2)

- Transport : UDP (`renet`), port de jeu par défaut **47800**, découverte LAN broadcast sur **47801**.
- `PROTOCOL_VERSION: u32` dans `mw_net`, vérifié au handshake ; bump à chaque changement de message.
- Serveur autoritaire, tick 60 Hz, snapshots ~30 Hz, prédiction client + réconciliation, interpolation ~100 ms.
- Sérialisation compacte (bincode ou équivalent) ; messages définis dans `mw_net::protocol`.
