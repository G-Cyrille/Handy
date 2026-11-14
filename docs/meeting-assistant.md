# Assistant de réunion « Fil ouvert »

La section Réunions de Handy devient une bibliothèque. Chaque réunion ouvre une
fenêtre dédiée, réutilisée quand on clique à nouveau sur la même réunion.
Fermer cette fenêtre ne coupe ni la capture ni les agents.

Préparation contient le titre, le dossier, l’objectif, les participants, les
documents, les réunions précédentes et le dossier de travail des agents.
Un chemin comme `Tibria/Interne` crée une hiérarchie logique sans déplacer les
archives audio. Le chat Principal conserve son historique et son brouillon en
passant de Préparation à En direct. On peut commencer cette conversation avant
l’enregistrement et la continuer après.

En direct permet de cocher des passages sans modificateur clavier, de joindre
une sélection au chat, de la corriger avec l’IA et de tout désélectionner.
Une note s’insère dans le fil, au début du dernier passage sélectionné ou à
l’instant courant. Les notes se modifient et se suppriment depuis ce fil.
La dictée n’intervient pas dans ce parcours.

Les captures sont manuelles. Le réglage initial est Tous les écrans ; les autres
choix sont l’écran sous le pointeur, une fenêtre d’application avec le sélecteur
macOS, et une zone. Elles apparaissent dans le fil et se joignent à la discussion
choisie. Une capture jointe est envoyée au modèle choisi lors de l’envoi du
message. Un enregistrement vidéo importé permet aussi d’extraire une image à un
timestamp, sans envoyer la vidéo.

Les petites actions défilent horizontalement au-dessus de la saisie. Leur croix
retire seulement la suggestion. Cliquer sur une action crée une discussion avec
son agent. Le bouton + crée une demande libre. La croix d’un onglet arrête cet
agent et conserve son historique. Une nouvelle consigne reprend la session du
binaire quand elle est disponible. Les consignes envoyées pendant un travail
sont conservées et exécutées dans l’ordre ; un arrêt ou une erreur annule la
file restante sans effacer les messages.

Les favoris sont des demandes personnalisées, une par ligne. Par exemple :
« Prépare le rapport du mois en cours, cite les sources puis partage le résultat
avec Share ». Leur exécution utilise les outils du binaire sélectionné.

Raccourcis dans la fenêtre :

- `⌘ J` : proposer des actions sur la sélection ou les dernières paroles.
- `⌘ ⇧ N` : insérer une note dans le fil.
- `⌘ ⇧ C` : prendre une capture manuelle.
- `⌘ Entrée` : envoyer le message ou enregistrer la note.
- `Échap` : fermer le panneau ouvert et désélectionner les passages.

## Modèles et outils

Codex et Claude passent directement par les exécutables installés. Handy ne copie
ni leur authentification ni leur configuration. Le transport utilise leurs modes
CLI JSON, sans T3 Code et sans dépendance ACP. Les modèles des providers HTTP
déjà configurés dans Handy restent accessibles pour le chat et les corrections.
Ils ne disposent pas des outils d’exécution des agents CLI.

Codex utilise `read-only` dans Principal et `workspace-write` avec réseau dans les
agents. Claude garde ses règles de permission existantes ; les demandes qui
nécessiteraient une confirmation sont refusées dans ce transport non interactif.
Les skills et outils installés restent ceux du binaire. Le dossier de travail se
choisit dans Préparation ; par défaut, les résultats vont dans le dossier de la
réunion. Le réglage Rapide réduit l’effort de raisonnement dans les binaires CLI
et demande sa désactivation aux providers HTTP compatibles. Il ne sélectionne
pas un palier de facturation prioritaire.

Certains catalogues Codex personnalisés contiennent des alias incomplets pour le
format CLI. Handy produit alors une copie compatible dans la réunion, en retirant
ces entrées de métadonnées. Le catalogue global, le provider et les identifiants
ne changent pas. La reprise passe les options après `resume`.
Voir la [configuration officielle Codex](https://developers.openai.com/codex/config-reference/).

Le contexte courant est borné aux 150 derniers segments, ou aux passages
explicitement sélectionnés, avec le résumé roulant, les notes et les documents
texte. Les réunions liées fournissent leurs derniers passages et notes.
Les agents CLI peuvent rechercher plus ancien avec la CLI locale. Les documents
non textuels nécessitent leurs outils de lecture. Le transport HTTP Anthropic
existant ne prend pas d’images ; Claude CLI et les providers compatibles vision
OpenAI peuvent recevoir les captures.

## Archives et imports

L’archive audio et l’ASR existants restent locaux. À l’arrêt, Handy génère
`meeting.mp3`, `transcript-original.json`, `transcript-corrected.json` et leurs
versions texte. Les textes sont actualisés lorsque l’ASR finit de rattraper la
réunion. Une erreur d’export est signalée et conserve les WAV sources.
Le bouton Exporter permet de régénérer les fichiers et d’ouvrir le dossier.
FFmpeg doit être installé pour le MP3, l’import audio/vidéo et l’extraction
d’images. Les fichiers importés sont copiés dans l’archive locale.

`workspace.json` conserve préparation, notes, références de captures, discussions,
brouillons, file de demandes, corrections et noms attribués aux passages. Les
écritures sont sérialisées et atomiques. Après une fermeture imprévue, les agents
en cours deviennent interrompus ; aucun travail ne redémarre automatiquement.

La séparation individuelle des voix distantes n’est pas ajoutée par cette
refonte : Microphone et Système restent des pistes audio, pas des identités de
participants. Les noms sont modifiables par passage. Les textes corrigés restent
séparés du texte ASR original.

## CLI de lecture

Chaque profil reçoit `meetings/meeting-query.py`. Elle lit SQLite en mode
lecture seule et ne récupère jamais les sessions actives comme interrompues.
Les commandes renvoient du JSON, utilisable par une autre IA :

```sh
python3 scripts/meetings/query.py --root '/chemin/vers/meetings' list
python3 scripts/meetings/query.py --root '/chemin/vers/meetings' --folder Tibria/Interne list
python3 scripts/meetings/query.py --root '/chemin/vers/meetings' search novembre
python3 scripts/meetings/query.py --root '/chemin/vers/meetings' show ID
```

La liste comprend titre, dossier, état et résumé roulant lorsqu’il existe.
`show` comprend transcription, notes, discussions, corrections et chemins locaux.
Cette CLI ne capture pas d’écran, n’arrête pas une réunion et ne lance pas d’agent.

## Vérification et profil indépendant

Les tests frontend utilisent le port 1442 et des fixtures synthétiques.
Le bundle `Handy Meeting Assistant Dev` utilise son propre profil
`com.pais.handy.meeting-assistant-dev`, son IPC et son chemin de build. Il conserve
la signature stable du bundle de test existant. Comme l’autre profil Réunions,
il partage donc son identité TCC, pas ses données. Il ne remplace pas l’app dans
`/Applications`.

```sh
bun run build
bun run lint
bun run check:translations
bunx playwright test --config playwright.meetings.config.ts
cargo test --manifest-path src-tauri/Cargo.toml --lib -- --test-threads=1
cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings
python3 scripts/meetings/build-test.py --assistant
open -n 'src-tauri/target/debug/bundle/macos/Handy Meeting Assistant Dev.app'
```

Attendre que la fenêtre soit prête avant un contrôle natif. Le contrôleur utilise
le socket de l’instance ouverte et ne lance jamais un second processus client :

```sh
python3 scripts/meetings/control.py --assistant '{"operation":"snapshot"}'
python3 scripts/meetings/assistant-smoke.py --with-codex
```

Le test natif crée une réunion synthétique dans ce seul profil, sans capturer
d’audio. Il utilise la configuration Codex existante pour vérifier une réponse,
la reprise de la même session, la création d’un fichier par un agent puis une
consigne ajoutée pendant son travail. Il conserve son rapport dans
`/private/tmp/handy-meeting-assistant-native.json`.

Validation du 14 novembre 2026 :

- Rust : 406 tests réussis, 20 ignorés ; Clippy sans avertissement.
- Interface : six tests Playwright, dont les deux tests du visualiseur de captures.
- Build signé, types TypeScript, lint, formatage et clés de traduction vérifiés.
- Test natif Codex réussi : réponse, reprise de session, fichier produit et
  consigne en attente exécutée une seule fois. Une seule instance principale
  du profil de test ; aucune capture audio démarrée.
- Fenêtre macOS vérifiée avec les résultats réels, les notes dans En direct et
  le redimensionnement compact.

Claude est implémenté mais n’a pas été lancé pendant cette validation. Le choix
Claude du test frontend utilise une fixture, sans appel au binaire. La capture
audio et les permissions en situation d’appel n’ont pas été retestées ici.
L’import et l’export audio utilisent des données synthétiques dans les tests Rust.
