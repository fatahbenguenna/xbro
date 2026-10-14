# Contribuer à xbro

Ce guide explique comment compiler, tester, vérifier visuellement et
soumettre une modification. La conception du moteur est décrite dans
[ARCHITECTURE.md](ARCHITECTURE.md) ; l'usage du navigateur dans
[README.md](README.md). Si vous venez de Java, commencez par les
[repères Rust](#repères-rust-pour-les-développeurs-java) en fin de document.

## Prérequis

- **Rust ≥ 1.85** via [`rustup`](https://rustup.rs), avec `rustfmt` et
  `clippy` (`rustup component add rustfmt clippy`). N'importe quelle version
  stable récente convient (le dépôt n'épingle pas de toolchain).
- **Un compilateur C** (`build-essential` sous Debian/Ubuntu), pour la crate
  `ring` utilisée par `rustls`.
- **Polices pour les tests** : les tests de texte, de mise en page et de rendu
  chargent les polices DejaVu depuis `/usr/share/fonts/truetype/dejavu`
  (paquet `fonts-dejavu-core`). Si elles manquent, ces tests se terminent
  sans rien vérifier (ceux de `xbro-text` affichent `skipping: … not
  installed`, les autres rien) : installez-les pour que la suite ait un
  sens. Quelques tests utilisent aussi, s'ils sont présents,
  `fonts-liberation`, `fonts-crosextra-carlito`, `fonts-noto-color-emoji` et
  `fonts-unifont`.
- **Pour l'interface graphique sans écran** (facultatif) : `xvfb`,
  `xdotool`, `imagemagick` et les bibliothèques d'exécution X11 listées dans
  le README.

Aucun accès à Internet n'est nécessaire pour compiler (une fois les crates
téléchargées) ni pour tester.

## Compiler

```sh
cargo check --workspace --all-targets   # vérification rapide, tests compris
cargo build                             # debug : target/debug/xbro
cargo build --release                   # optimisé (LTO « thin ») : target/release/xbro
```

Le profil `dev` compile les crates du dépôt en `opt-level = 1` et les
dépendances en `opt-level = 3` (décodage d'images, TLS et rastérisation sont
trop lents sinon). Les pages lourdes restent nettement plus lentes en debug :
utilisez `--release` pour juger des performances ou naviguer.

Le workspace a `crates/xbro` comme membre par défaut : sans `--workspace` ni
`-p`, `cargo build` et `cargo test` ne concernent que le binaire.

La documentation des API (commentaires `///` et `//!`) se consulte avec
`cargo doc --workspace --no-deps --open`.

## Tester

### Commandes

```sh
cargo test --workspace                        # toute la suite (≈1 500 tests)
cargo test -p xbro-css                        # une crate
cargo test -p xbro-layout --test grid         # un fichier de tests d'intégration
cargo test -p xbro-style cascade              # les tests dont le nom contient « cascade »
cargo test -p xbro-engine --test fixtures -- --nocapture   # afficher les println!
```

### Organisation des tests

- **Tests unitaires** : dans un module `#[cfg(test)] mod tests` en bas du
  fichier, ou dans un sous-module `tests.rs` (par exemple
  `crates/xbro-css/src/selector/tests.rs`). Ils ont accès aux éléments
  privés.
- **Tests d'intégration** : `crates/<crate>/tests/*.rs`, qui n'utilisent que
  l'API publique ; les aides partagées sont dans `tests/common/mod.rs`
  (`tests/support/mod.rs` pour `xbro-css`).
- **Pages de référence** : `tests/fixtures/*.html` (+ `wiki.css`,
  `wiki-import.css`, `img/`), utilisées par les tests de `xbro-layout`,
  `xbro-paint`, `xbro-engine` et `xbro`. Plusieurs tests listent les
  fixtures en dur (cherchez `"wiki.html"`) : ajoutez-y une nouvelle page.
- **HTTP** : `crates/xbro-net/tests/` et `crates/xbro-engine/tests/http.rs`
  démarrent des serveurs locaux (`TcpListener` sur `127.0.0.1:0`) ; les
  hôtes locaux ne passent jamais par un proxy.
- **Ligne de commande** : `crates/xbro/tests/headless.rs` exécute le binaire
  compilé (captures, dumps, codes de sortie, interface sans écran qui échoue
  proprement).
- **Interface** : le modèle `Browser` est testé sans fenêtre
  (`crates/xbro/src/gui/*.rs`) ; `gui/stress_tests.rs` lui envoie de longues
  séquences aléatoires d'événements.

### Tests aléatoires (« fuzz »)

Il n'y a pas de `cargo fuzz` : les tests aléatoires sont des tests ordinaires,
alimentés par de petits générateurs pseudo-aléatoires à graine fixe
(xorshift), donc **reproductibles** et exécutés à chaque `cargo test` :

| Crate | Fichier | Ce qui est vérifié |
|---|---|---|
| `xbro-html` | `tests/robustness.rs` | balisage aléatoire, octets aléatoires sous divers encodages, balisage réel muté : arbre toujours bien formé |
| `xbro-css` | `tests/serialization_fuzz.rs` | `to_css` relu redonne les mêmes valeurs |
| `xbro-net` | tests unitaires (`input`, `mime`, `data_url`, `http`…) | saisies, en-têtes et URL arbitraires |
| `xbro-style` | `tests/robustness.rs`, `tests/regressions.rs` | documents et feuilles aléatoires ; dupliquer une feuille ne change rien |
| `xbro-text` | `tests/properties.rs`, `tests/hostile_fonts.rs` | invariants du shaping, polices corrompues |
| `xbro-layout` | `tests/robustness.rs` | documents aléatoires, valeurs hostiles, imbrications profondes sur petite pile |
| `xbro-paint` | `tests/image_decode.rs` | images corrompues ou tronquées |
| `xbro-engine` | `tests/fuzz_pipeline.rs` | pipeline complet : documents et feuilles aléatoires chargés depuis `file:`, redimensionnés, cliqués, survolés, saisis et rendus, sans panic, sans géométrie non finie, en temps et mémoire bornés |
| `xbro` | `src/gui/stress_tests.rs` | séquences d'événements de l'interface |

D'autres campagnes s'ajoutent au fil du temps : cherchez `fuzz` ou `Rng`
dans `crates/*/tests/`.

Des variables d'environnement allongent ou varient certaines campagnes (la
documentation en tête de chaque fichier fait foi) :

```sh
# xbro-style : nombre de tours, graine, dossier où écrire le cas en échec
XBRO_FUZZ_ROUNDS=5000 cargo test -p xbro-style --release --test robustness
XBRO_FUZZ_SEED=12345 cargo test -p xbro-style --test robustness
XBRO_DUMP_DIR=/tmp/xbro-scratch cargo test -p xbro-style --test regressions
# xbro-engine : campagne longue (5 000 documents, ignorée par défaut)
cargo test -p xbro-engine --release --test fuzz_pipeline -- --ignored --nocapture
# xbro-engine : rejouer ou isoler un document (XBRO_FUZZ_SEED, XBRO_FUZZ_DOCS,
# XBRO_FUZZ_ONLY, XBRO_FUZZ_DUMP, XBRO_FUZZ_REPLAY)
XBRO_FUZZ_ONLY=42 cargo test -p xbro-engine --test fuzz_pipeline -- --nocapture
```

### Tests de performance

Les budgets ne sont vérifiés qu'en `--release` (en debug, les tests
s'exécutent avec des bornes larges ou affichent seulement leurs mesures) :

```sh
# Page synthétique de ~1 Mo : budget 1,5 s pour chargement + une rastérisation
cargo test -p xbro-engine --release --test perf -- --nocapture
# Mise en page : 15 000 nœuds en moins de 200 ms, tableaux, flex/grille imbriqués…
cargo test -p xbro-layout --release --test perf
# Cascade d'un grand document (ignoré en debug)
cargo test -p xbro-style --release --test perf
# Mesure d'un million de caractères
cargo test -p xbro-text --release --test perf
# Banc de rastérisation (ignoré par défaut) : image 1280×800 en moins de 15 ms
cargo test -p xbro-paint --release --test perf -- --ignored --nocapture
# Banc sur une page « réaliste » de ~1,5 Mo (ignoré par défaut)
cargo test -p xbro-engine --release --test bench_realworld -- --ignored --nocapture
```

Les tests longs, les bancs d'essai et les rendus de contrôle (voir plus
bas) sont marqués `#[ignore]`, le plus souvent avec une raison qui indique
comment les lancer ; `grep -rnE '#\[(cfg_attr\(.*)?ignore' crates/` les
liste, et `cargo test --workspace` les compte comme « ignored ».

## Lint et formatage

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Les deux doivent être propres avant toute soumission (`-D warnings` fait de
chaque avertissement une erreur). Le formatage est celui de `rustfmt` par
défaut (pas de `rustfmt.toml`).

## Vérifier visuellement une modification de rendu

Un test automatique ne voit pas tout : pour toute modification de style, de
mise en page ou de peinture, comparez le rendu des fixtures avant et après.
Travaillez hors du dépôt (par exemple dans `/tmp/xbro-scratch/`) ; le
`.gitignore` ignore de toute façon les `*.png` hors de `tests/fixtures/` et
`docs/`.

```sh
mkdir -p /tmp/xbro-scratch/avant /tmp/xbro-scratch/apres
for f in tests/fixtures/*.html; do
  n=$(basename "$f" .html)
  cargo run --release -q -- --screenshot /tmp/xbro-scratch/avant/$n.png -w 1024 -H 0 "$f"
done
# … appliquer la modification, puis la même boucle vers apres/ …
for f in /tmp/xbro-scratch/avant/*.png; do
  n=$(basename "$f")
  printf '%s : ' "$n"
  compare -metric AE "$f" /tmp/xbro-scratch/apres/$n /tmp/xbro-scratch/diff-$n 2>&1; echo
done
```

`compare` (ImageMagick) affiche le nombre de pixels différents et produit une
image des différences ; ouvrez les PNG pour juger. Autres aides :

- `--dump-layout` donne l'arbre de boîtes (positions et tailles) : un `diff`
  avant/après localise un changement de géométrie ;
- `--scale 2` vérifie le rendu HiDPI, `-w 420` une mise en page étroite ;
- `XBRO_DUMP=<dossier>` fait écrire des PNG à certains tests de
  `xbro-paint` (`visual_dump`, `end_to_end`, `fixture_pages`) et de
  `xbro-engine` (aide `dump` de `tests/common`) ;
- `cargo test -p xbro-engine --test visual_dump -- --ignored` rend toutes
  les fixtures (à 1024 et 420 px de large), les pages générées (erreurs,
  texte, image seule) et des états de focus dans `$XBRO_REVIEW_DIR` (par
  défaut `/tmp/xbro-verify/engine-review`).

## Lancer l'interface graphique sous Xvfb

Sans écran (serveur, CI, conteneur), l'interface se pilote sur un écran
virtuel :

```sh
Xvfb :99 -screen 0 1280x900x24 &
export DISPLAY=:99
./target/release/xbro -w 1280 -H 860 tests/fixtures/wiki.html &
sleep 3
WID=$(xdotool search --name xbro | head -1)
xdotool windowfocus "$WID"            # sans gestionnaire de fenêtres, donner le focus soi-même
xdotool key ctrl+f                    # touches envoyées par XTEST
xdotool type "Rust"
xdotool key Return
xdotool mousemove --window "$WID" 65 189 click 1
import -window "$WID" /tmp/xbro-scratch/gui.png   # capture de la fenêtre
xdotool key ctrl+w                    # fermer le dernier onglet quitte xbro
```

- N'utilisez pas `xdotool key --window …` : il passe par `XSendEvent`, dont
  `winit` ignore les événements ; donnez le focus puis envoyez les touches
  sans `--window`.
- `-w` et `-H` fixent la taille initiale de la fenêtre en px logiques ; les
  captures de `docs/screenshots/` font 1280×860.
- Sans `DISPLAY` ni `WAYLAND_DISPLAY`, xbro s'arrête avec `cannot open a
  window (…); use --screenshot for headless rendering`.

## Conventions

### Ne jamais paniquer sur du contenu web

Toute donnée venue d'une page (HTML, CSS, images, polices, en-têtes HTTP,
URL) est hostile par hypothèse. Un panic tue le thread (et, dans le thread de
l'interface, le navigateur).

- Pas d'`unwrap()`, d'`expect()` ni d'indexation `v[i]` sur ces données :
  `get()`, `if let`/`let … else`, `?`. `expect` n'est acceptable que sur une
  constante vérifiée par un test (voir `constant_url` dans `xbro-net`).
- Arithmétique : `saturating_*`/`checked_*` sur les entiers ; sur les
  flottants, remplacer NaN et borner (`clamp`) avant de s'en servir comme
  taille, position ou nombre d'itérations.
- Tout ce qui grandit avec l'entrée a une borne : une constante `MAX_…`
  documentée, et un test qui la dépasse. Pas de récursion non bornée (la
  profondeur du DOM est limitée à 256) ; préférez une pile explicite.
- Coût linéaire ou quasi linéaire : une page de 1 Mo ne doit pas exploser en
  temps. Ajoutez un test de performance si un algorithme peut devenir
  quadratique.
- Pas de `todo!()` ni d'`unimplemented!()` : une fonctionnalité non gérée
  est ignorée proprement (et documentée comme approximation).
- `catch_unwind` reste un filet de sécurité aux frontières existantes, pas
  une façon de traiter les erreurs.

### Tests

- Chaque modification s'accompagne de tests **dans la crate concernée** ; une
  correction de bug commence par un test qui le reproduit.
- Préférez des assertions précises (positions, tailles, couleurs de pixels,
  arbre produit) ; `LayoutTree::outline` donne une vue compacte de l'arbre de
  boîtes pour comparer des mises en page.
- Les tests ne doivent dépendre ni d'Internet, ni de l'horloge (sauf tests de
  performance, avec des marges), ni de fichiers hors du dépôt autres que les
  polices système ; un test qui a besoin d'une police absente doit se
  terminer sans échouer.

### Code et API

- Code, identifiants et commentaires en anglais ; la documentation Markdown
  est en français. Chaque module commence par un commentaire `//!` qui
  explique son rôle, ses choix et ses approximations : tenez-le à jour.
- Les API publiques sont le contrat entre crates : les étendre est permis,
  les changer impose d'adapter tous les appelants dans le même changement.
- Les dépendances externes sont déclarées une seule fois dans
  `[workspace.dependencies]` du `Cargo.toml` racine et référencées par
  `nom.workspace = true`. N'ajoutez pas de crate externe sans discussion
  préalable : le moteur est écrit à la main par principe.
- Une nouvelle limite, une nouvelle approximation ou une limitation levée se
  reporte dans le README (« Limites connues ») et dans ARCHITECTURE.md.

### Avant de soumettre

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Pour un changement de rendu, joignez des captures avant/après des fixtures
concernées ; pour un changement sensible aux performances, les mesures des
tests `perf` en `--release`.

## Repères Rust pour les développeurs Java

| Rust | Équivalent Java approximatif |
|---|---|
| crate | module Maven / JAR |
| workspace (`Cargo.toml` racine) | projet Maven multi-modules (POM parent) |
| `Cargo.toml` d'une crate | `pom.xml` d'un module |
| `mod` | package |
| `pub(crate)` | visibilité limitée à la crate (proche de « package-private ») |
| `struct` + bloc `impl` | classe (données + méthodes), sans héritage |
| `trait` | interface (avec méthodes par défaut possibles) |
| `enum` avec données + `match` | `sealed interface` + `record` + `switch` exhaustif |
| `Option<T>` | `Optional<T>`, mais il n'existe pas de `null` |
| `Result<T, E>` et `?` | exception vérifiée, explicite dans le type de retour ; `?` la propage |
| `panic!` | `RuntimeException` non rattrapée : arrête le thread (interdit sur contenu web) |
| `catch_unwind` | `try { … } catch (Throwable t)` en dernier recours |
| `&T` / `&mut T` | référence en lecture partagée / en écriture exclusive, vérifiée à la compilation |
| `Arc<T>` | référence partagée entre threads, comptée (libérée au dernier `drop`) |
| `Mutex<T>`, `RwLock<T>` | `synchronized`, `ReentrantReadWriteLock` ; le verrou contient la donnée protégée |
| `Send` / `Sync` | « transférable à un autre thread » / « partageable entre threads », vérifiés par le compilateur |
| `Vec<T>`, `HashMap<K, V>` | `ArrayList<T>`, `HashMap<K, V>` |
| `#[cfg(test)] mod tests` | tests JUnit du même package |
| `tests/*.rs` | tests d'intégration (API publique uniquement) |
| `cargo clippy` / `cargo fmt` | analyse statique (≈ SpotBugs, Sonar) / formateur (≈ Spotless) |

Deux idées reviennent partout dans le code :

- **Pas de graphe d'objets avec pointeurs** : le DOM est une arène
  (`Vec<Node>`) et les nœuds se désignent par leur index (`NodeId`), comme si
  l'on stockait des identifiants plutôt que des références dans une liste
  Java. C'est ce qui permet de partager et d'indexer facilement les données.
- **Pas d'exceptions** : une opération qui peut échouer renvoie `Option` ou
  `Result`, et le moteur choisit toujours un repli (page d'erreur, valeur par
  défaut, élément ignoré) plutôt que d'interrompre le rendu.
