# Installation de l’assistant de réunion

Installation sur ce Mac le 14 novembre 2026, à la demande de Cyrille.

- Application : `/Applications/Handy Screenshot Test.app`, version `0.10.1`.
- Sources installées : `ea334d4f`, depuis le worktree canonique `HandyFork-personal`
  et sa branche `personal/main`.
- Refonte Réunions : `93cf9577`, avec la correction du titre de bibliothèque
  `e7cc848f`.
- Upstream intégré : `aca89851`, release 0.10.1.
- Branche d’intégration : `chore/integrate-meeting-assistant-20261114`.

Le build réutilise `src-tauri/tauri.screenshot-test.conf.json`, son identifier
`com.pais.handy.screenshot-test` et son identité Apple Development existante.
La designated requirement et les entitlements correspondent au bundle précédent.
La signature et le SHA-256 du binaire installé ont été vérifiés. Aucun reset TCC,
aucune signature ad hoc et aucun changement d’authentification des agents.

Les déclenchements OpenLogi ont été suspendus par Cyrille avant le remplacement.
Handy a été quitté par son menu natif ; les copies du profil ont été faites app
fermée. Le premier lancement de chaque bundle a utilisé LaunchServices.

## Vérifications

- 408 tests Rust réussis, 20 ignorés ; Clippy avec `-D warnings` réussi.
- Six tests Playwright réussis ; lint, types TypeScript, format et traductions
  vérifiés. Après la correction du titre, lint et build signé repassés, avec
  vérification des libellés statiques français et anglais.
- Bibliothèque et ouverture d’une archive dans sa fenêtre dédiée vérifiées dans
  l’application installée. Titre, transcription, chat Principal et choix du
  binaire Codex visibles.
- 138 entrées d’historique, 23 réunions, 6 398 segments et 46 pistes conservés,
  avec égalité des valeurs de toutes les colonnes antérieures.
- 6 697 fichiers existants de `recordings` et `meetings/sessions` inchangés par
  SHA-256.
- Réglages conservés, sauf le passage du schéma à la version 3 et l’arrondi f32
  du volume, inférieur à `0.000001`.

L’agent Codex réel avait été validé dans le profil indépendant avant installation,
avec reprise de session, fichier produit et consigne en attente exécutée une seule
fois. Aucune requête IA ni capture audio n’a été lancée dans le profil quotidien
pour cette installation. Les autorisations audio et capture restent donc à
confirmer par un usage réel ; la signature stable ne suffit pas à les prouver.

## Sauvegarde et retour arrière

Sauvegarde locale complète :

```text
/Users/cyrillegrossholtz/Documents/dev/AI/HandyFork/target/handy-screenshot-backups/20261114-205028-meeting-assistant-jefk3nez
```

Elle conserve le bundle précédent, le profil avant installation et une seconde
copie après le premier lancement. `baseline.json` et `verification.json`
contiennent les empreintes et compteurs de préservation, sans texte transcrit ni
credential. Le profil sauvegardé contient les données privées habituelles et
reste local dans un dossier réservé à l’utilisateur.

Un retour arrière nécessite de suspendre à nouveau les déclenchements, quitter
Handy, sauvegarder l’état courant, puis remettre le bundle précédent au même
chemin et le lancer via macOS. Ne pas écraser automatiquement le profil courant :
il peut contenir des réunions créées depuis cette installation.

Les limites fonctionnelles figurent dans [le guide de l’assistant](meeting-assistant.md),
notamment l’absence de diarisation individuelle automatique.
