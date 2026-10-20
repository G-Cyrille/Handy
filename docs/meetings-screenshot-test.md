# Réunions dans Handy Screenshot Test

Installation demandée et effectuée le 20 octobre 2026.
Code installé : `8237703`, branche `feat/meetings-screenshot`.

## Version utilisée pour la suite

- App : `/Applications/Handy Screenshot Test.app`.
- Sources : `/Users/cyrillegrossholtz/Documents/dev/AI/HandyFork/target/meetings-worktree`.
- Profil : `~/Library/Application Support/com.pais.handy.screenshot-test`.
- Branche combinant `feat/meetings-v1` et `personal/main`, sans modifier ces
  deux branches ni le checkout initial sur `feat/voxtral-q4`.
- Contexte par screenshot, curseur et focus, historique des images et Voxtral Q4
  conservés, avec Réunions, ses exports et les huit entrées du menu.

Le worktree est durable, sous le dossier `target/` ignoré du checkout initial.
L'ancien worktree de `/tmp` avait perdu ses sources lors d'un nettoyage.

## Construction et mise à jour

Depuis le worktree durable, avec sa configuration personnelle ignorée copiée
depuis le checkout initial :

```sh
bun run tauri build --debug \
  --config src-tauri/tauri.screenshot-test.conf.json \
  --bundles app
```

Conserver le nom, le bundle identifier et l'identité de signature de cette
configuration. Le produit `Handy Screenshot Test` utilise son profil habituel ;
le produit `Handy Meetings Dev` reste isolé par `test_build::isolate_config`.
Les scripts `scripts/meetings/build-test.py` et `control.py` concernent toujours
le profil isolé et ne pilotent pas l'app quotidienne installée.

Après construction, quitter seulement Screenshot Test, sauvegarder son bundle et
son profil, copier le nouveau bundle, vérifier sa signature puis le lancer avec
`open -n '/Applications/Handy Screenshot Test.app'`. Aucune remise à zéro TCC.

## Vérification du 20 octobre

- `cargo test --lib` : 326 réussis, 0 échec, 16 ignorés.
- `cargo clippy -- -D warnings` : réussi pour les cibles livrées.
- `cargo clippy --all-targets -- -D warnings` relève quatre lints préexistants
  dans les tests : `nonminimal_bool`, `manual_repeat_n`,
  `items_after_test_module` et `write_with_newline`. Ils restent hors de
  cette intégration.
- Lint frontend, TypeScript/Vite, format et traductions : réussis.
- Les deux tests Playwright existants du viewer de screenshots : réussis,
  avec un serveur de fixture propre au worktree sur le port 18439.
- Bundle installé vérifié par `codesign --verify --deep --strict`, avec une
  designated requirement identique à celle de l'ancien bundle.
- Interface native : Meetings et les anciens menus présents, réglages de
  screenshots et Voxtral Q4 présents, sources de réunion consultables.
  Aucune capture ni requête IA démarrée pour cette installation.
- Les 530 lignes d'historique et les 1 301 fichiers d'enregistrement existants
  correspondent aux empreintes relevées avant installation. Les réglages actifs
  sont conservés. La sérialisation supprime l'ancien champ inutilisé
  `ort_accelerator` et conserve le volume à son arrondi f32 près.
- Les 23 sessions de l'ancien profil Réunions sont copiées dans le profil
  Screenshot Test, avec égalité des tables `sessions` et `segments`. Le profil
  source reste conservé.

Sauvegarde locale de l'ancienne app et du profil complet :
`/Users/cyrillegrossholtz/Documents/dev/AI/HandyFork/target/handy-screenshot-backups/20261020-155233`.
Le fichier `verification.json` de ce dossier contient les compteurs et le bilan
de préservation, sans transcript ni credential.

Le fetch du 20 octobre donne `main...upstream/main = 0 / 2` ; upstream est
`93c9ee0`. Aucun rebase, changement de `main` ou push. Les limites de capture
longue restent celles de `meetings-v1-validation.md` : quatre heures, les vrais
appels Meet/kMeet et le provider réel ne sont pas revalidés ici.
