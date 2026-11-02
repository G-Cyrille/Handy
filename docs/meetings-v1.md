# Réunions V1

Branche `feat/meetings-v1`, basée sur `upstream/main` à `4150a4a`.
Capture audio macOS 15 minimum. Pas de vidéo, de diarization individuelle ni d’exécution d’actions IA.

## Lancer le build isolé sur ce Mac

Depuis `/private/tmp/HandyFork-meetings` :

```sh
python3 scripts/meetings/prepare-test.py
python3 scripts/meetings/build-test.py
open -n 'src-tauri/target/debug/bundle/macos/Handy Meetings Dev.app'
```

Le script de build lit la configuration personnelle ignorée du checkout initial,
sans l’écrire ni afficher son identité de signature. À la demande de Cyrille,
le bundle réutilise la coquille signée `com.pais.handy.screenshot-test` et ses
permissions macOS existantes. Le nom reste **Handy Meetings Dev**.

Avant l’initialisation de Tauri, le profil Réunions prend l’identifiant runtime
`com.pais.handy.meetings-dev`. Données, historique, logs, IPC et raccourcis restent
séparés. Le test `screenshot_shell_isolates_only_the_meeting_profile` couvre cette
distinction. `open -n` lance une instance séparée malgré l’identité macOS partagée.
Ne pas copier ce bundle dans `/Applications`.

- Données : `~/Library/Application Support/com.pais.handy.meetings-dev`.
- Logs : `~/Library/Logs/com.pais.handy.meetings-dev`.
- Port dev : 1438.
- Dictée test : Ctrl+Alt+Shift+Cmd+D ; réunion : Ctrl+Alt+Shift+Cmd+M.
- Autostart et vérification de mises à jour désactivés dans ce profil.
- Les credentials du seul provider quotidien sélectionné peuvent être lus en place,
  sans être copiés ni écrits. Aucun historique quotidien n’est importé.

Arrêter seulement la capture, puis seulement le build de test :

```sh
python3 scripts/meetings/control.py '{"operation":"stop"}'
python3 scripts/meetings/control.py '{"operation":"quit"}'
```

L’IPC de test exige une instance déjà lancée par Finder/LaunchServices et n’existe
pas en release. Son import WAV est limité aux fixtures sous
`/private/tmp/handy-meetings-fixtures`.

## Parcours et stockage

Dans Meetings, choisir le micro et le son système ou une application. Un filtre
d’application inclut tous ses onglets et fenêtres. L’IA est facultative et utilise
le provider/modèle affiché ; ses prompts sont distincts de ceux de la dictée.
L’écran reste allumé pendant la capture pour préserver le filtre ScreenCaptureKit.
Cette protection contre la veille automatique se termine avec la capture, y
compris après une erreur. Un verrouillage manuel, une veille forcée ou la
fermeture du capot peuvent interrompre la capture ; l’archive déjà écrite reste
disponible. Aucun réglage global d’alimentation n’est modifié.

Apple Intelligence n’est pas proposé pour les réunions : son bridge ne permet pas
d’annuler les requêtes à la clôture d’une session.

La capture écrit deux pistes WAV mono 16 kHz en fenêtres de cinq secondes et un
index SQLite indépendant. Les silences restent dans l’archive. Les compteurs
distinguent audio reçu, silence ajouté, overlap et samples perdus. Une erreur
d’écriture arrête la session avec une erreur ; une erreur d’ASR ou d’IA conserve
l’audio. Après crash, une session active revient comme interrompue.

L’ASR local consomme les fichiers finalisés. Son scheduler donne la priorité à la
dictée sans arrêter la capture. Le transcript de réunion ne passe pas dans le
pipeline de collage. Le VAD économise l’inférence sans supprimer l’audio ; Retry
permet de forcer les fenêtres classées sans parole, en conservant les textes déjà
finalisés et en évitant les fenêtres composées uniquement de padding.

Sessions permet la consultation, la lecture des segments et l’export M4A AAC :
micro à gauche, son système/application à droite. L’export crée temporairement un
WAV stéréo, soit environ 922 Mo pour quatre heures, en plus de l’archive source.

## Transcript et suggestions IA

Dans Sessions, ouvrir une réunion arrêtée puis choisir Markdown, texte brut ou
JSON à côté d’Exporter la transcription. Le fichier complet `transcript.md`,
`transcript.txt` ou `transcript.json` est écrit dans le dossier de la session,
puis sélectionné dans Finder. L’export utilise une lecture cohérente de toute
l’archive, indépendamment de la page affichée. Les timestamps relatifs et les
pistes sont conservés. Les fenêtres ASR encore en attente ou en erreur restent
signalées ; on peut refaire l’export après rattrapage. Les fenêtres sans parole
sont omises dans les versions lisibles et conservées avec leur statut dans le
JSON. L’audio source n’est pas réécrit.

Le JSON version 1 contient les métadonnées `session` et un tableau `segments`,
avec `start_ms`, `end_ms`, `track`, `status`, `text` et `error`. Les timestamps
peuvent avoir une partie décimale ; `track` vaut `microphone` ou `system`.
Ces pistes ne sont pas des identités de participants : la V1 ne fait pas de
diarization individuelle des personnes distantes.

AI suggestions transmet les nouveaux segments de texte finalisés et un résumé
roulant au provider/modèle configuré dans Post Process. Aucun audio n’est envoyé
par ce chemin. Au maximum une requête toutes les trente secondes, avec des
fenêtres de contexte bornées, produit un résumé et jusqu’à trois suggestions.
Les suggestions n’exécutent aucune action. Le résumé est consultable dans la
session archivée. Désactiver le toggle arrête les requêtes et annule la tâche
côté app, sans arrêter l’ASR local ni effacer les résultats déjà enregistrés.
Après trois échecs consécutifs, les requêtes s’arrêtent ; désactiver puis
réactiver le toggle permet de retenter. Ce chemin reste à valider sur le
serveur FAST configuré, qui était inaccessible pendant les premiers essais.

Les entrées General, History, Models, Advanced, Post Process, Debug et About
restent toutes visibles, en plus de Meetings. Afficher Post Process ou Debug
n’active pas leur option correspondante.

## Vérifications reproductibles

```sh
bun run format:check
bun run lint
bun run check:translations
bun run build
cargo test --offline --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1
cargo clippy --offline --manifest-path src-tauri/Cargo.toml --lib
python3 scripts/meetings/test-audio-converter.py
```

Le test Rust `binding_export_tests::export_typescript_bindings` régénère
`src/bindings.ts`, fichier généré exclu du formatter frontend.
Le bundle Réunions ne modifie pas ce fichier à son lancement.

Pour une source contrôlée audio seule :

```sh
python3 scripts/meetings/make-fixtures.py
python3 scripts/meetings/build-fixture-player.py
open '/private/tmp/Handy Fixture Player.app'
```

Le lecteur rejoue la fixture française de 45 secondes et écrit son heure de
démarrage dans `player-start.json`, à côté du WAV. Sélectionner **Handy Fixture
Player** comme source. Cette méthode vérifie ScreenCaptureKit et l’ASR natifs ;
elle ne valide pas Google Meet, kMeet ou le filtrage d’un navigateur.

Après avoir démarré une réunion native avec deux pistes :

```sh
python3 scripts/meetings/endurance.py --seconds 14400 --interval 30 --verify-fixture-signal \
  --output /private/tmp/handy-meetings-endurance.jsonl
```

Le moniteur ne lance ni n’arrête Handy. Il consigne RSS et CPU fournis par `ps`,
taille d’archive, espace libre, backlog, buffers, durée/retard ASR, progression des
transcripts, compteurs des pistes et erreurs. `completed` signifie que la durée de
mesure est atteinte ; il faut encore vérifier les marqueurs audio et l’export.
L’option de contrôle du signal exige la source Handy Fixture Player. Elle échoue
si le signal connu de cette fixture disparaît des trois dernières fenêtres ; elle
ne s’applique pas à une réunion ordinaire. L’état numérique du lecteur est aussi consigné. Dans le seul bundle de test,
les diagnostics PCM avant/après conversion et de reconfiguration passent par
la queue bornée vers le log Rust, sans texte transcrit ni samples audio.
Le test court de reconstruction native
est `python3 scripts/meetings/recovery.py --microphone 'MacBook Pro Microphone'`.
Ne pas uploader les enregistrements. Les résultats mesurés sont consignés dans le
rapport de validation de la branche.
