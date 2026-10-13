# xbro

**xbro** est un navigateur web écrit en Rust, **avec son propre moteur de rendu**.
Le parseur HTML, le parseur CSS, la cascade, la mise en page et la
rastérisation sont implémentés de zéro dans ce dépôt ; seules les briques
système (TLS/HTTP, polices, décodage d'images, fenêtre) viennent de crates
externes.

> Il n'y a pas de JavaScript : xbro affiche le Web « document » (Wikipédia,
> Hacker News, blogs, documentation, DuckDuckGo HTML…), les balises
> `<noscript>` sont donc rendues.

## Fonctionnalités

**Moteur**

- HTML : tokenizer et construction d'arbre façon WHATWG (fermetures implicites,
  tableaux, éléments de formatage mal imbriqués, SVG en ligne, entités, détection
  de l'encodage).
- CSS : sélecteurs niveau 4 (`:is`, `:where`, `:not`, `:has`, `:nth-child`…),
  `@media`, `@supports`, `@import`, imbrication CSS, variables (`var()`),
  `calc()`/`min()`/`max()`/`clamp()`, raccourcis, attributs de présentation HTML.
- Mise en page : bloc (fusion des marges), texte (coupure de lignes,
  justification, `white-space`, `vertical-align`), flottants, positionnement
  relatif/absolu/fixe, listes, flexbox, tableaux (`colspan`/`rowspan`),
  approximation de la grille, images, SVG, contrôles de formulaire,
  `::before`/`::after`.
- Peinture : ordre d'empilement et `z-index`, `opacity`, `overflow`, bordures
  (styles et coins arrondis), dégradés linéaires, images de fond, texte
  anti-crénelé avec repli de police par caractère.
- Réseau : HTTP/1.1 + TLS (rustls), redirections, cookies, gzip/brotli,
  `file:`, `data:`, pages internes `about:home` et `about:help`.

**Navigateur**

- Onglets, historique précédent/suivant, barre d'adresse (URL, domaine, chemin
  local ou recherche DuckDuckGo).
- Liens, ancres `#id`, formulaires GET/POST (champs texte, cases à cocher,
  boutons radio, listes déroulantes), `<meta http-equiv=refresh>`.
- Défilement (molette, clavier, barre), zoom, recherche dans la page.
- Modes sans fenêtre : capture PNG, dump du DOM ou de l'arbre de mise en page.

## Compiler et lancer

Prérequis : Rust ≥ 1.85 (`rustup`). Aucune bibliothèque système n'est
nécessaire à la compilation ; à l'exécution, l'interface graphique utilise
X11 ou Wayland sous Linux, et fonctionne aussi sous macOS et Windows.

```sh
cargo run --release                       # ouvre about:home
cargo run --release -- https://en.wikipedia.org/wiki/Rust_(programming_language)
cargo run --release -- tests/fixtures/modern.html
```

### Modes sans fenêtre

```sh
# Capture de la page entière (hauteur 0) en 1280 px de large
cargo run --release -- --screenshot page.png --width 1280 --height 0 news.ycombinator.com
cargo run --release -- --dump-dom tests/fixtures/basic.html
cargo run --release -- --dump-layout tests/fixtures/basic.html
```

## Raccourcis clavier

| Action | Raccourci |
|---|---|
| Barre d'adresse | `Ctrl+L` (puis `Entrée`) |
| Précédent / suivant | `Alt+←` / `Alt+→`, boutons latéraux de la souris |
| Recharger | `F5`, `Ctrl+R` |
| Nouvel onglet / fermer | `Ctrl+T` / `Ctrl+W` |
| Onglet suivant / précédent | `Ctrl+Tab` / `Ctrl+Shift+Tab`, `Ctrl+1…9` |
| Rechercher dans la page | `Ctrl+F` (`Entrée` / `Shift+Entrée`, `Échap`) |
| Zoom | `Ctrl++`, `Ctrl+-`, `Ctrl+0` |
| Défiler | molette, flèches, `Espace`, `Page↑/↓`, `Début/Fin` |
| Parcourir les liens et champs | `Tab` / `Shift+Tab`, `Entrée` pour activer |
| Ouvrir un lien dans un nouvel onglet | `Ctrl+clic` ou clic du milieu |

## Architecture

Le dépôt est un workspace Cargo, une crate par étape du pipeline :

```
xbro-net ─► xbro-html ─► xbro-dom ─► xbro-style (+ xbro-css) ─► xbro-layout (+ xbro-text) ─► xbro-paint
                                     └──────────────── xbro-engine (Page) ─────────────────┘
                                                            xbro (fenêtre + CLI)
```

Voir [ARCHITECTURE.md](ARCHITECTURE.md) pour les contrats entre crates.

## Tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

`tests/fixtures/` contient des pages représentatives (style Wikipédia,
Hacker News, mise en page moderne flex/grid, formulaires, tableaux, texte).

## Limites connues

- Pas de JavaScript, pas de cache HTTP, pas de HTTP/2.
- Pas de mise en forme de texte complexe (arabe, hébreu, indien) ni de texte
  bidirectionnel ; pas d'emoji en couleur.
- Grille CSS approximée, pas de transformations, d'animations ni d'ombres.
- Pas de défilement interne des éléments `overflow: auto`.

## Licence

MIT ou Apache-2.0, au choix.
