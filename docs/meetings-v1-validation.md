# Réunions V1 : validation du 15 octobre 2026

**Version de code : `666f00cbfb5b590f8aedf75de1beff2b262984cd`, branche `feat/meetings-v1`.**
Backend : `2c1bc1c`. Aucune modification de `personal/main`, aucun push.
Base : `4150a4a`, `main` sans divergence avec `upstream/main` lors du dernier contrôle.

**Verdict provisoire : capture et transcription natives vérifiées pour un essai
court. L’endurance de quatre heures a échoué et demande un correctif de capture.**
Les suggestions sur le provider configuré et les vrais appels Meet/kMeet restent
à valider.

## Endurance interrompue

La session `1792077680052`, démarrée à 17 h 21, a conservé 5 640,351 secondes,
soit environ 1 h 34. À 18 h 55, le débranchement d’un périphérique de capture a
coïncidé avec le retrait de Display 3 dans WindowServer. ScreenCaptureKit a
terminé le stream avec `Failed to find any displays or windows to capture`.
Le filtre d’application était ancré sur un display choisi une fois au démarrage.
L’écran interne restait disponible. Le correctif doit conserver le même micro et
le même PID source en reconstruisant la capture sur un display survivant.

Mesures avant l’arrêt : RSS app médian 49,2 Mio, maximum 55,5 Mio ; RSS worker
médian 1 619,3 Mio, maximum 1 628,7 Mio ; buffers maximum 191 334 samples ;
backlog maximum 40 secondes, puis nul après rattrapage. Les 2 258 segments sont
traités, dont 1 406 avec résultat ASR et 852 sans parole. Les deux pistes ont zéro
sample perdu ou dupliqué signalé. L’archive reste disponible.

Un premier essai de 13 minutes avait déjà été interrompu : les samples de la piste
système sont devenus nuls à 17 h 12 min 50 s, au moment d’un changement de route
audio associé à une dictée quotidienne. Le lecteur continuait à jouer. Un nouveau
stream a rétabli le signal. Les zéros ayant été mesurés après conversion, leur
origine exacte dans ScreenCaptureKit ou le convertisseur reste à isoler.

Ces essais ne valident pas quatre heures et ne sont pas additionnés pour produire
un résultat d’endurance. La tâche de mesure a été arrêtée avant les corrections.

### Correctif et tests courts de reprise

Le filtre privilégie désormais un écran interne actif. La disparition de son
affichage, une erreur native `-3815` ou une reconfiguration matérielle audio
déclenchent une reconstruction bornée du stream, avec le même PID et le même UID
micro. Deux tentatives maximum, notifications coalescées, callbacks de l’ancien
stream ignorés et arrêt prioritaire. Le silence seul ne déclenche aucune reprise.
Les interruptions restent comptées comme padding dans l’archive et une notice
est affichée.

La suite passe à 290 tests Rust réussis, aucun échec, 16 ignorés ; Clippy et le
formatter passent. Les 9 cas du convertisseur Swift et les compilations ARM64 et
Intel passent. Le bundle signé a passé le test natif de trois demandes de refresh
avec signal présent avant/après, sources inchangées et arrêt concurrent à une
reprise. Session `1792084354873` : zéro sample perdu ou dupliqué signalé ; padding
micro 24 634 samples et système 22 448, pré-roll inclus.

Le BY-V était absent lors de ce nouveau test ; le micro intégré a été sélectionné
explicitement. L’app ne bascule pas automatiquement vers un autre microphone
quand le microphone choisi est débranché. Le test de refresh forcé ne remplace pas
un nouveau test physique de débranchement ou une transition Bluetooth réelle.
Les diagnostics numériques avant/après conversion permettent désormais de
localiser une éventuelle nouvelle panne de PCM. Le moniteur peut aussi vérifier
le signal connu de la fixture avec `--verify-fixture-signal`, sans traiter les
silences ordinaires d’une réunion comme des pannes.

La nouvelle revue externe du correctif a été refusée par l’approbation automatique
pour transmission de détails non publics. Revue locale et tests ci-dessus
effectués ; aucune nouvelle revue externe réussie à compter pour ce correctif.

## Environnement et isolation

- Mac M1 Pro, macOS 26.6.2, build 25G83.
- Modèle local : Voxtral Mini 4B Realtime 2602, GGUF Q4_K_M, cache existant en lecture seule.
- Provider sélectionné : Custom, modèle FAST ; IA désactivée pour les captures natives.
- Build : `/private/tmp/HandyFork-meetings/src-tauri/target/debug/bundle/macos/Handy Meetings Dev.app`.
- À la demande de Cyrille, identité macOS de la coquille Screenshot Test conservée.
  Profil runtime `com.pais.handy.meetings-dev` distinct pour données, logs et IPC.
- Handy quotidien `/Applications/Handy Screenshot Test.app` resté au PID initial 78333. Empreinte de son `settings_store.json` inchangée lors des contrôles.
- Checkouts `feat/voxtral-q4` et `personal/main` propres et préservés.

Le [runbook](meetings-v1.md) décrit le build, le lancement séparé, les fixtures,
l’arrêt ciblé et les commandes de vérification.

## Résultats acquis

| Vérification                                           | Résultat                                                                                                                               |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| Rust, exécution série                                  | 289 réussis, 0 échec, 16 ignorés                                                                                                       |
| Clippy, lint, formatter, traductions, TypeScript/build | Réussis                                                                                                                                |
| Bundle macOS signé                                     | Build debug natif et vérification codesign réussis                                                                                     |
| Convertisseur Swift réel                               | 8 cas réussis : planar/interleaved, 48/44,1/16 kHz, downmix, dérive ±1000 ppm                                                          |
| Archive synthétique française, 45 s × 2 pistes         | 720 000 samples par piste ; pertes, overlaps et padding nuls                                                                           |
| ASR réel avec VAD                                      | « Oui », « Non », « D’accord », affirmation et négation conservées                                                                     |
| Export synthétique via l’app                           | AAC 48 kHz, 2 canaux, 45,056 s pour 45 s source                                                                                        |
| Capture native micro intégré + lecteur dédié           | Signal audio distant et transcript français vérifiés                                                                                   |
| Capture native BY-V + lecteur dédié                    | 188,839 s, deux pistes, pertes/overlaps nuls                                                                                           |
| Export de cette capture native                         | AAC 48 kHz, 2 canaux, 188,885 s                                                                                                        |
| Disparition de l’application source                    | Arrêt avec erreur visible, 68,108 s d’archive conservées                                                                               |
| Handler de raccourci Réunions                          | 2 cycles start/stop ; répétition et relâchement n’arrêtent pas la capture                                                              |
| Coexistence native                                     | 3 débuts de dictée, 2 annulations et 1 finalisation ; capture continue ; presse-papiers témoin inchangé puis contenu original restauré |

Le test de coexistence n’a pas fourni de phrase parlée contrôlée : il ne valide
pas le contenu d’une dictée collée. Une tentative de raccourci physique n’a pas
déclenché la session ; le log signale SecureInput externe, qui bloque les touches.
Le test du handler IPC ne remplace pas cette vérification physique.

Les pistes natives contiennent environ une seconde de pré-roll conservateur à
l’ouverture, puis les éventuels écarts de callbacks sont comptés comme silence
ajouté. Pour l’essai BY-V de 188,839 s : micro 3 001 383 samples reçus et 20 041
ajoutés ; système 3 002 560 reçus et 18 864 ajoutés. Aucune perte/duplication
signalée. Ce padding n’est pas une suppression de l’archive.

## Crashs et erreurs

- Crash réel pendant un import synthétique accéléré : les quatre segments déjà
  durables sont récupérés avec leurs empreintes intactes, session interrompue.
  Ce cas ne mesure pas la perte temporelle d’une capture native.
- Crash réel pendant l’inférence sur fixture : 18/18 fichiers audio inchangés ;
  reprise de l’ASR après relance, sans réimport de la source.
- **Crash natif deux pistes à 18,167 s**, ASR en cours : 15 s par piste récupérées,
  tous les hashes durables conservés. **Perte maximale mesurée : 3,167 s par piste**,
  sous la cible de 10 s pour ce crash de processus. Pas de test de coupure électrique.
- Erreur d’écriture, récupération d’orphelins, buffers bornés, priorité ASR, retry
  VAD, erreurs/timeout HTTP et déduplication des suggestions couverts par les tests
  ciblés. Un véritable ENOSPC sur volume borné reste à exercer.
- La première suite Rust parallèle avait révélé un timeout du test clipboard
  préexistant ; sa reprise isolée et la suite complète série passent.

## Limites de validation

- Le provider configuré échoue depuis l’app avec `No route to host`. Les tests de
  transport HTTP local, y compris 503 et timeout, passent ; ils ne valident pas
  FAST sur le serveur configuré. Aucun enregistrement natif envoyé à l’IA.
- Apple Intelligence n’est pas supportée pour les suggestions Réunions, faute
  d’annulation native des requêtes. Les providers HTTP existants sont réutilisés.
- Le lecteur natif dédié constitue la source contrôlée. Les navigateurs temporaires
  ont échoué à rendre leur page locale ; leurs premières pistes silencieuses ne
  sont pas comptées comme validation du son d’un navigateur.
- Google Meet, kMeet, exclusion entre deux applications sonores, débranchement et
  rebranchement physiques du micro restent à tester.
- La capture système Windows/Linux n’est pas implémentée ; des stubs conditionnels
  conservent l’interface. Aucune cross-compilation Windows/Linux faite sur ce Mac.

L’approbation automatique a refusé un clic d’Accessibilité pour le nouveau bundle,
la capture du Chromium quotidien susceptible d’inclure des onglets privés, puis
une dictée sans cible de collage confirmée. Les alternatives utilisées sont la
coquille Screenshot Test expressément demandée, une source synthétique dédiée et
un éditeur de test avec sauvegarde/restauration du presse-papiers en mémoire.

## Preuves locales

Artefacts volumineux conservés hors Git et sans upload d’enregistrement :

- `/private/tmp/handy-meetings-french-final.json` : ASR de la fixture finale.
- `/private/tmp/handy-meetings-native-player-final.json` : transcript du son réellement capturé.
- `/private/tmp/handy-meetings-native-source-exit.json` : panne de source et compteurs.
- `/private/tmp/handy-meetings-byv-final.json` : capture et compteurs du micro externe.
- `/private/tmp/handy-meetings-native-shortcut-cycles.json` : cycles du handler.
- `/private/tmp/handy-meetings-coexistence-result.json` : continuité pendant dictée/annulation.
- `/private/tmp/handy-meetings-native-crash.json` : horloge, segments et empreintes avant/après crash.
- `/private/tmp/handy-meetings-shell-rust-tests.log` et `handy-meetings-shell-clippy.log` : checks Rust.
- `/private/tmp/handy-meetings-converter-final.log` : mesures du convertisseur Swift.
- `/private/tmp/handy-meetings-endurance*` : montage, télémétrie et résultat de l’endurance.

## Répartition du travail

Sol et Opus : exploration UI proportionnée. Sol high : capture native. Opus high :
archive puis UI. Fable high : deux revues indépendantes, puis exécution de
l’endurance. Le parent Sol high a intégré, corrigé les défauts des revues et
exécuté les vérifications ci-dessus. Aucune escalade de modèle pendant cette
reprise ; aucune validation simulée présentée comme capture native.
