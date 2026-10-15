# Réunions V1 : validation du 15 octobre 2026

**Code corrigé, construit et lancé : `37fa0ab`, branche `feat/meetings-v1`.**
Endurance interrompue sur `db6d3f5` ; le correctif de veille n’a pas encore
traversé une nouvelle endurance.
Backend : `2c1bc1c`. Aucune modification de `personal/main`, aucun push.
Base : `4150a4a`. Après le fetch du 15 octobre au soir, `upstream/main` a un
commit supplémentaire, `9e57a0d`, qui modifie le chunking et l’ASR. Cette branche
conserve sa base pour ne pas mélanger ce changement aux mesures ; aucun rebase
ni changement de `main` effectué pendant la validation.

**Verdict provisoire : capture et transcription natives vérifiées pour un essai
court. La nouvelle endurance s’est arrêtée après 1 h 17 min 47 s lors de la
veille automatique de l’écran. Quatre heures ne sont pas validées.**
Les suggestions sur le provider configuré et les vrais appels Meet/kMeet restent
à valider.

## Endurance interrompue

La session `1792077680052`, démarrée à 17 h 21, a conservé 5 640,351 secondes,
soit environ 1 h 34. Cyrille a confirmé avoir débranché le microphone BY-V
sélectionné à 18 h 55, et non un périphérique vidéo ou un écran. L’arrêt sur
perte du microphone choisi est attendu : aucun fallback automatique vers un autre
microphone ne doit modifier les sources capturées.

ScreenCaptureKit a terminé le stream avec
`Failed to find any displays or windows to capture`. WindowServer a aussi
journalisé le retrait de Display 3. Cette observation concomitante ne prouve pas
que Cyrille a débranché un écran ni que ce retrait a causé l’arrêt. Le message
natif ne permettait pas d’identifier clairement la perte du BY-V. Le correctif de
reconstruction décrit ci-dessous couvre les reconfigurations de capture ; il ne
remplace pas un microphone débranché.

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

### Nouvelle endurance interrompue par la veille de l’écran

La session `1792084550673` a démarré à 19 h 15 min 50 s avec le micro intégré
explicitement sélectionné, le lecteur synthétique dédié, deux pistes et l’ASR
local actif. Elle teste le commit `db6d3f5`, avec vérification périodique du
signal connu. L’archive dure **4 667,382 s, soit 1 h 17 min 47 s**. Le moniteur
sort avec le code 1 et `completed: false`, sans atteindre les 14 400 s demandées.

À 20 h 33 min 18 s, powerd signale l’expiration de `UserIsActive` après une
heure d’inactivité. À 20 h 33 min 28 s, WindowServer publie `Broadcast: Will
Sleep` et loginwindow déclenche `kLWLockFromDisplayDim`. La reprise du bridge
échoue ensuite avec `No active display is available for the system audio
filter`. Il s’agit cette fois d’une veille automatique suivie du verrouillage,
avec le micro intégré toujours sélectionné. Le helper `caffeinate` prévu par le
montage était déjà absent au premier contrôle de l’agent de suivi.

Sur les 154 relevés, RSS app médiane 48,9 Mio et maximum 93,0 Mio ; RSS worker
médiane 1 634,9 Mio et maximum 1 640,4 Mio. Maximum de buffers : 190 807 samples.
Le signal synthétique est présent dans les fenêtres contrôlées jusqu’à l’arrêt.
Le backlog cumulé des deux pistes atteint 680 s ; le dernier retard de segment
ASR mesuré atteint environ 355 s. Le zéro affiché par le snapshot après arrêt
n’est pas un rattrapage complet : 129 segments étaient encore en attente au
moment de ce relevé. Au contrôle final, 456,9 s après l’arrêt IPC, les
1 868 segments sont traités : 1 728 final et 140 sans parole, zéro erreur ASR.
Ce rattrapage dépasse la limite de contrôle de 120 s. Cette cadence ne valide pas une transcription proche du
temps réel sur deux pistes continuellement parlées.

Les 934 fenêtres par piste couvrent la même durée, sans gap ni overlap dans
l’index et sans fichier WAV manquant. À l’arrêt, les compteurs signalent zéro
sample perdu ou dupliqué ; le padding atteint 9,409 s pour le micro et 9,322 s
pour le système, dont environ 8,3 s à la fin. Les contrôles numériques de la
fixture au début, au milieu et à la fin donnent un décalage stable de 18 ms,
avec des corrélations d’enveloppes de 0,9702, 0,9724 et 0,8790. Cela ne constitue
pas une réécoute humaine. L’export termine en AAC stéréo 48 kHz, 4 667,435 s,
environ 64,4 Mo. Son IPC dépasse 55 s, puis une réponse tardive confirme le
succès ; aucune seconde exportation n’a été lancée.

Le correctif `37fa0ab` garde une activité macOS pendant la capture pour empêcher
la veille automatique de l’écran et du système. Son arrêt, y compris après une
erreur de démarrage, libère l’activité. Un test natif interroge les assertions
IOPM et confirme leur présence puis leur libération. Les neuf tests du
convertisseur restent verts. Lint, TypeScript/build, clés de traduction,
formatter, `cargo check --lib` et compilation Swift Intel passent également. Une indication dans le choix des sources explique
que l’écran reste allumé et qu’un verrouillage ou une veille peut interrompre
la capture. Aucun réglage global de veille n’est modifié, aucun verrouillage
manuel n’est contourné. Un test court du bundle et une nouvelle endurance
complète restent nécessaires pour qualifier ce correctif. Le Mac est ensuite
vérifié verrouillé, avec zéro écran actif ; aucune nouvelle capture n’est
lancée dans cet état. Le bundle corrigé signé passe `codesign --verify`, puis
est lancé via LaunchServices au PID 31244. Le snapshot confirme le stockage
isolé et aucune capture active. Le test court de capture reste en attente du
déverrouillage du Mac.

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

Le runbook `docs/meetings-v1.md` décrit les fixtures et les commandes de
vérification. Depuis `/private/tmp/HandyFork-meetings`, pour reconstruire et lancer :

```sh
python3 scripts/meetings/build-test.py
open -n 'src-tauri/target/debug/bundle/macos/Handy Meetings Dev.app'
```

Pour arrêter uniquement la capture puis le build de test :

```sh
python3 scripts/meetings/control.py '{"operation":"stop"}'
python3 scripts/meetings/control.py '{"operation":"quit"}'
```

## Résultats acquis

| Vérification                                           | Résultat                                                                                                                               |
| ------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- |
| Rust, exécution série                                  | 290 réussis, 0 échec, 16 ignorés                                                                                                       |
| Clippy, lint, formatter, traductions, TypeScript/build | Réussis                                                                                                                                |
| Bundle macOS signé                                     | Build debug natif et vérification codesign réussis                                                                                     |
| Convertisseur Swift réel                               | 9 cas réussis : planar/interleaved, 48/44,1/16 kHz, downmix, dérive ±1000 ppm, reset avec gap                                          |
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
- Google Meet, kMeet et exclusion entre deux applications sonores restent à
  tester. Le retrait du BY-V a interrompu l’ancienne endurance ; le message
  d’erreur spécifique et le cycle complet de reconnexion restent à valider.
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
- `/private/tmp/handy-meetings-endurance-v2-result.md` et `.json` : analyse de l’essai interrompu de 1 h 17 min 47 s.
- `/private/tmp/handy-meetings-sleep-converter.log` : neuf cas PCM et assertions de veille.
- `/private/tmp/handy-meetings-sleep-rust-check.log` : intégration du correctif Swift/Rust.
- `/private/tmp/handy-meetings-sleep-build-verification.json` : nouveau bundle lancé, profil isolé, absence de capture et préservation quotidienne.

## Répartition du travail

Sol et Opus : exploration UI proportionnée. Sol high : capture native. Opus high :
archive puis UI. Fable high : deux revues indépendantes et première endurance.
Codex Astra high : suivi et analyse de l’endurance `1792084550673`. Le parent a
intégré, corrigé les défauts et exécuté les vérifications du correctif de veille.
Aucune validation simulée présentée comme capture native.
