# Mettre à jour le fork Handy

Le fork GitHub de Cyrille est `G-Cyrille/Handy`. Récupérer les changements ou
créer un commit local ne met pas ce dépôt GitHub à jour : il faut aussi pousser
les branches vers `origin`.

## Remotes et branches

```text
cjpais/handy                 G-Cyrille/Handy
upstream/main --fetch--> main --push--> origin/main
                         |
                         merge avec les fonctionnalités du fork
                         |
                    personal/main --push--> origin/personal/main
```

- `upstream` désigne le projet original. Son push URL reste `DISABLED`.
- `origin` désigne le fork de Cyrille et reçoit tous les push.
- `main` reste une copie de `upstream/main`, sans commits propres au fork.
- `feat/*`, `fix/*` et `refactor/*` isolent les changements indépendants.
- `personal/main` rassemble les fonctionnalités du fork et les mises à jour
  upstream. C'est la branche à consulter sur GitHub pour la version combinée.
- `chore/sync-upstream-*` prépare une intégration sans modifier directement
  `personal/main`. `chore/update-screenshot-test` a servi à l'intégration
  initiale des screenshots, de Voxtral Q4 et de Réunions.

Le worktree principal `HandyFork` sert au développement. Le worktree voisin
`../HandyFork-personal` garde `personal/main` disponible pour construire la
version combinée. Les anciens worktrees de fonctionnalités ne sont pas la
source canonique du fork.

## Autorisation et périmètre

Une demande de mise à jour de Handy, d'intégration upstream ou de synchronisation
du fork autorise les commits et push nécessaires à ce workflow. Une demande
explicitement locale s'arrête avant le push. Une simple question sur Git ne
déclenche aucun changement.

Les force-push et les contributions au projet original, PR, issues et
discussions, demandent toujours une autorisation distincte. Utiliser des merges
pour préserver l'historique publié. Si un fast-forward ou un push échoue,
examiner la divergence avant de poursuivre ; ne pas reset une branche pour la
faire correspondre à un remote.

Ce workflow est exécuté à chaque demande de mise à jour. Ce n'est pas une
synchronisation planifiée. Il suit le dernier `upstream/main` disponible au
fetch, pas seulement la dernière release GitHub. Une release publiée et le
dernier code de `main` peuvent être différents.

## 1. Récupérer upstream et origin

Depuis le worktree principal :

```sh
git status --short --branch
git worktree list
git remote -v
git fetch origin --prune
git fetch upstream --prune
git rev-list --left-right --count main...upstream/main
```

Préserver tout travail en cours. Les worktrees concernés doivent être propres
avant un changement de branche ou un merge. Ne pas créer un stash ni supprimer
des fichiers utilisateur pour débloquer automatiquement l'opération.

Si le worktree d'intégration n'existe pas encore, le créer avec
`git worktree add ../HandyFork-personal personal/main`. Si la branche locale
n'existe pas, la créer d'abord avec
`git branch --track personal/main origin/personal/main`. Puis vérifier son état
avec `git -C ../HandyFork-personal status --short --branch`.

Mettre à jour la copie de l'original avec des fast-forwards uniquement :

```sh
git switch main
git merge --ff-only origin/main
git merge --ff-only upstream/main
git -C ../HandyFork-personal merge --ff-only origin/personal/main
```

`main...upstream/main` doit alors donner `0 0`.

## 2. Intégrer sans perdre les fonctionnalités du fork

Créer une branche d'intégration avec une date ou un nom unique :

```sh
git switch -c chore/sync-upstream-YYYYMMDD personal/main
git merge --no-ff --no-commit main
```

Résoudre les conflits en conservant les changements upstream et les
fonctionnalités du fork. Vérifier aussi les fichiers mergés automatiquement,
les bindings générés, les traductions et les migrations de données.

Pour intégrer une nouvelle fonctionnalité terminée, merger sa branche dans
cette même branche d'intégration. Les changements indépendants se développent
sur des branches issues de `main`, pas directement sur `personal/main`.

Si `main` est déjà inclus, Git ne crée pas de merge en attente : ne pas créer
un commit vide. Si un conflit révèle un bug dans une fonctionnalité isolée,
corriger d'abord sa branche puis intégrer cette correction.

## 3. Vérifier et committer

Installer les dépendances depuis le lockfile et lancer les vérifications
adaptées au diff :

```sh
bun install --frozen-lockfile
bun run lint
bun run format:check
bun run build
bun run check:translations
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
git diff --check
git diff --name-only --diff-filter=U
```

La dernière commande ne doit rien afficher. Régénérer `src/bindings.ts` via
tauri-specta si les commandes ou types Rust changent. Lancer les tests frontend
concernés. Un changement de documentation seul ne nécessite pas de reconstruire
le moteur Rust.

Stager explicitement les fichiers résolus, examiner le diff staged puis créer
le commit de merge avec un message conventionnel. Des modifications documentaires
indépendantes peuvent avoir leur propre commit.

## 4. Publier le résultat sur le fork GitHub

Après validation, depuis le worktree principal, remplacer le nom d'exemple
par celui de la branche d'intégration :

```sh
git -C ../HandyFork-personal status --short --branch
git -C ../HandyFork-personal merge --ff-only chore/sync-upstream-YYYYMMDD
git push --atomic origin main personal/main chore/sync-upstream-YYYYMMDD
git branch --set-upstream-to=origin/chore/sync-upstream-YYYYMMDD
```

Le push atomique publie les trois branches ensemble ou n'en publie aucune.
Pousser uniquement `main` oublie les fonctionnalités du fork ; pousser uniquement
une branche de travail laisse `personal/main` obsolète. Ne pas pousser les tags
upstream ni créer une release automatiquement.

Vérifier le résultat après le push :

```sh
git fetch origin --prune
git rev-list --left-right --count main...origin/main
git rev-list --left-right --count personal/main...origin/personal/main
git merge-base --is-ancestor upstream/main personal/main
git ls-remote origin refs/heads/main refs/heads/personal/main
git status --short --branch
git -C ../HandyFork-personal status --short --branch
```

Les deux compteurs doivent donner `0 0`, le test d'ascendance doit réussir et les
SHA distants doivent correspondre aux branches locales. Rapporter le commit
upstream intégré, les commits publiés, les checks et les divergences restantes.

## Installation macOS, seulement sur demande

Une synchronisation Git ne remplace pas l'app installée. Pour mettre à jour
`/Applications/Handy Screenshot Test.app`, partir du `personal/main` validé et
utiliser la configuration ignorée `src-tauri/tauri.screenshot-test.conf.json`.
Elle est propre à chaque worktree : copier la configuration existante si elle
manque, en conservant son bundle identifier et son identité Apple Development.

Suivre [la procédure de build et de sauvegarde](meetings-screenshot-test.md).
Comparer les designated requirements, sauvegarder l'app et son profil, quitter
uniquement l'app concernée, installer et vérifier le bundle signé, puis le lancer
normalement via macOS. Ne pas remplacer le build combiné par une release
officielle qui ne contient pas les fonctionnalités du fork. Les resets TCC ne
font pas partie d'une mise à jour normale.
