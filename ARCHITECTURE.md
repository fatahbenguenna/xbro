# Architecture de xbro

xbro est un navigateur web écrit en Rust **avec son propre moteur de rendu** :
parseur HTML, parseur CSS, cascade, mise en page (layout) et rastérisation
sont implémentés dans ce dépôt. Seules les briques « système » viennent de
crates externes : TLS/HTTP (`ureq` + `rustls`), découverte et rastérisation
des polices (`fontdb`, `ab_glyph`), décodage d'images (`image`, `resvg`),
dessin 2D (`tiny-skia`), fenêtre (`winit` + `softbuffer`) et presse-papiers
(`arboard`). Il n'y a **pas de JavaScript** (les `<noscript>` sont donc rendus).

## Pipeline

```
 URL ──► xbro-net ──► octets ──► xbro-html ──► Document (xbro-dom)
                                                    │
         feuilles CSS (<style>, <link>, @import) ──► xbro-css ──► Stylesheet
                                                    │
                     xbro-style (cascade) ◄─────────┘
                           │  StyleMap (ComputedStyle par nœud)
                           ▼
                     xbro-layout (+ xbro-text pour mesurer le texte)
                           │  LayoutTree (boîtes + lignes + fragments)
                           ▼
                     xbro-paint ──► DisplayList ──► tiny_skia::Pixmap
                           ▲
 xbro-engine orchestre tout (Page) ; xbro (binaire) = interface graphique + CLI
```

## Crates et dépendances

| Crate | Rôle | Dépend de |
|---|---|---|
| `xbro-dom` | Arbre DOM en arène (`Document`, `NodeId`) | — |
| `xbro-html` | Décodage des octets, tokenizer, construction de l'arbre | dom |
| `xbro-css` | Tokenizer, parseur de feuilles, sélecteurs, media queries, couleurs | — |
| `xbro-style` | Cascade, héritage, valeurs calculées, feuille UA, attributs de présentation | dom, css |
| `xbro-text` | Polices système, sélection, fallback, shaping simple, glyphes | — |
| `xbro-net` | http(s), file:, data:, about:, saisie de la barre d'adresse | — |
| `xbro-layout` | Arbre de boîtes, bloc, inline, floats, positionnement, flex, tableaux | dom, css, style, text |
| `xbro-paint` | Display list, rastérisation, images | dom, css, style, text, layout |
| `xbro-engine` | Chargement de page, sous-ressources, interactions (liens, formulaires, recherche) | tout |
| `xbro` | Binaire : fenêtre (onglets, historique, barre d'adresse) et modes headless | engine, net, paint, text |

## Conventions transverses (le « contrat »)

* **Unités** : tout est en *CSS px* (`f32`). La conversion vers les pixels
  physiques (HiDPI × zoom) ne se fait qu'à la rastérisation (`RasterOptions::scale`).
* **Coordonnées** : le layout produit des rectangles **absolus** dans le repère
  du document (origine en haut à gauche du bloc conteneur initial). Exception :
  les sous-arbres `position: fixed` sont en coordonnées de viewport ; la display
  list les encadre de `PushFixed`/`PopFixed` et le rasteriseur n'y applique pas
  le défilement.
* **Identité des nœuds** : `NodeId` est un index stable dans l'arène du
  `Document` ; les styles (`StyleMap`) et les boîtes y font référence.
* **Styles** : les longueurs absolues sont résolues en px à la cascade ; les
  pourcentages restent des `LengthPercentage { px, percent }` (ce qui représente
  aussi `calc()`), résolus par le layout. `currentcolor` est résolu.
* **Robustesse** : aucune étape ne doit paniquer, quelle que soit l'entrée
  (HTML/CSS malformés, images corrompues, pages énormes). Le constructeur
  d'arbre HTML **limite la profondeur** de l'arbre à 256 éléments imbriqués
  (au-delà, les éléments sont ajoutés au nœud de profondeur 256), ce qui
  protège les étapes récursives (cascade, layout, peinture).
* **Performance** : une page de ~1 Mo (≈15 000 nœuds, ex. un article Wikipédia)
  doit passer parse → style → layout → display list en moins d'une seconde en
  `--release`. La rastérisation ne dessine que la zone visible.
* **Threads** : `FontSystem` et `Fetcher` sont `Send + Sync` (partagés via `Arc`),
  `Page` est `Send` (construite sur un thread de chargement, utilisée par l'UI).

## Détails par étape

### HTML (`xbro-html`)
Tokenizer conforme dans l'esprit au WHATWG (états data, RCDATA, RAWTEXT,
script, PLAINTEXT, attributs, commentaires, doctype, références de caractères
nommées et numériques). Constructeur d'arbre avec les modes d'insertion
principaux, fermetures implicites (`p`, `li`, `dt/dd`, `option`, `tr/td/th`,
titres…), éléments vides, adoption simplifiée des éléments de formatage,
tableaux (foster parenting simplifié), contenu étranger SVG/MathML (casse des
noms d'éléments et d'attributs SVG ajustée : `viewBox`, `linearGradient`…,
balises auto-fermantes respectées). Scripting désactivé.

### CSS (`xbro-css`) et style (`xbro-style`)
Syntaxe CSS 3 (récupération d'erreurs), sélecteurs niveau 4 (combinateurs,
attributs, `:nth-*`, `:not/:is/:where/:has`, pseudo-éléments `::before/::after`),
`@media`, `@supports`, `@import`, `@layer` (appliqué à plat). La cascade gère
les origines (UA / auteur), `!important`, l'attribut `style`, la spécificité,
l'héritage, `inherit/initial/unset/revert`, les propriétés personnalisées
(`--x` + `var()`), `calc()/min()/max()/clamp()`, les raccourcis
(`margin`, `border`, `font`, `background`, `flex`, `list-style`…) et les
attributs de présentation HTML (`bgcolor`, `width`, `align`, `<font>`…).

### Layout (`xbro-layout`)
Bloc (fusion des marges), inline (coupure de lignes, `white-space`,
`text-align` dont `justify`, `vertical-align`), floats et `clear`,
positionnement relatif/absolu/fixe (`sticky` ≈ relatif), listes et marqueurs,
flexbox (direction, wrap, grow/shrink/basis, justify/align, gap, order),
tableaux (largeur auto, `colspan`/`rowspan`, `border-spacing`), approximation de
la grille, éléments remplacés (images, SVG en ligne, contrôles de formulaire),
contenu généré. Les tailles intrinsèques (min/max-content) servent aux floats,
inline-blocks, cellules et éléments flex.

### Peinture (`xbro-paint`)
Ordre de peinture CSS 2.1 (annexe E) simplifié, `z-index`, `opacity`,
`overflow` (découpage), bordures (styles, rayons), dégradés linéaires, images
de fond (`repeat`, `size`, `position`), texte avec cache de glyphes,
soulignement/barré, images matricielles et SVG.

### Moteur (`xbro-engine`) et application (`xbro`)
Chargement parallèle des sous-ressources, pages d'erreur, documents non HTML,
`<base>`, `<meta refresh>`, liens, ancres `#id`, formulaires GET/POST,
champs texte, cases à cocher, recherche dans la page, `:hover`. L'application
offre onglets, historique précédent/suivant, barre d'adresse (URL ou recherche),
défilement (molette, clavier, barre), zoom, recherche (Ctrl+F) et des modes
headless (`--screenshot`, `--dump-dom`, `--dump-layout`).

## Règles de contribution

* `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` et
  `cargo test --workspace` doivent passer.
* Chaque crate a ses tests unitaires ; `tests/fixtures/` contient des pages HTML
  utilisées par les tests d'intégration du moteur et les captures d'écran.
* Les API publiques documentées dans les squelettes sont le contrat entre
  crates : les étendre est permis, les changer impose d'adapter les appelants.
