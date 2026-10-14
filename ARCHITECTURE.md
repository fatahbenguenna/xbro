# Architecture de xbro

Ce document décrit la conception de xbro : le pipeline de rendu, le rôle et
les types principaux de chaque crate, les algorithmes et les décisions de
conception, ainsi que les limites de robustesse. Il s'adresse à un
développeur qui veut lire ou modifier le code ; pour compiler, tester et
contribuer, voir [CONTRIBUTING.md](CONTRIBUTING.md), qui contient aussi des
repères Rust pour les développeurs venant de Java.

Le code, ses identifiants et ses commentaires sont en anglais. La
documentation de chaque module (`//!` en tête de fichier) détaille ce qui
est résumé ici : c'est la référence en cas de doute.

## Vue d'ensemble

xbro possède **son propre moteur de rendu** : décodage et parseur HTML,
parseur CSS, sélecteurs, cascade, mise en page, liste d'affichage et
rastérisation (au-dessus de `tiny-skia`) sont écrits dans ce dépôt. Seules les
briques « système » viennent de crates externes :

| Besoin | Crate externe | Utilisée par |
|---|---|---|
| HTTP/1.1, TLS, cookies, décompression | `ureq` (+ `rustls`, racines `webpki-roots`) | `xbro-net` |
| Analyse d'URL | `url` | `xbro-net`, `xbro-engine`, `xbro` |
| Encodages de caractères | `encoding_rs` | `xbro-html`, `xbro-net` |
| Découverte des polices | `fontdb` | `xbro-text` |
| Lecture des polices, contours de glyphes | `ab_glyph` | `xbro-text` |
| Décodage PNG/JPEG/GIF/WebP/BMP/ICO | `image` | `xbro-paint` |
| SVG (analyse et rendu) | `resvg` / `usvg` (sans la fonctionnalité `text`) | `xbro-paint` |
| Dessin 2D anti-crénelé | `tiny-skia` | `xbro-paint`, `xbro` |
| Fenêtre, clavier, souris | `winit` | `xbro` |
| Affichage d'un tampon de pixels dans la fenêtre | `softbuffer` | `xbro` |
| Presse-papiers | `arboard` | `xbro` |

Il n'y a **pas de JavaScript** : le parseur HTML fonctionne « scripting
désactivé » (le contenu de `<noscript>` est donc analysé et rendu), les
`<script>` sont ignorés.

## Pipeline

```
 URL ──► xbro-net ──► octets ──► xbro-html ──► Document (xbro-dom)
                                                    │
         feuilles CSS (<style>, <link>, @import) ──► xbro-css ──► Stylesheet
                                                    │
                     xbro-style (cascade) ◄─────────┘
                           │  StyleMap (un Arc<ComputedStyle> par nœud rendu)
                           ▼
                     xbro-layout (+ xbro-text pour mesurer le texte)
                           │  LayoutTree (boîtes + lignes + fragments)
                           ▼
                     xbro-paint ──► DisplayList ──► tiny_skia::Pixmap
                           ▲
 xbro-engine orchestre tout (Engine::load → Page) ; xbro (binaire) = interface graphique + CLI
```

Un chargement (`xbro_engine::Engine::load`, fonctions `loader::load` puis
`loader::build_page`) enchaîne :

1. **fetch** : `Fetcher::fetch` (ou `post_form` pour un formulaire POST)
   rend une `Response` (URL finale après redirections, statut, type MIME,
   corps) ou une `NetError`, qui devient une page d'erreur ;
2. **classification** : HTML (et XHTML), texte (`text/*`, JSON, XML, scripts…
   affichés dans un `<pre>`), image (affichée centrée) ou type non affichable
   (page « cannot display ») ; un statut HTTP ≥ 400 avec un corps vide donne
   une page d'erreur générée ;
3. **parse** : `xbro_html::decode_html_bytes` puis `parse_document` ;
4. **stylesheets** : `<style>` et `<link rel=stylesheet>` dans l'ordre du
   document (attribut `media` respecté), feuilles externes et `@import`
   récupérés en parallèle, URL rendues absolues ;
5. **style** : `xbro_style::compute_styles` ;
6. **images** : découverte (`<img>`, `<picture>`, `srcset`, `<input
   type=image>`, `<video poster>`, `<object>`/`<embed>`, `<svg>` en ligne,
   images CSS) puis chargement parallèle sous budget ;
7. **layout** : `xbro_layout::layout` ;
8. **display list** : `xbro_paint::build_display_list_full`, puis ajout de
   l'anneau de focus et du curseur de texte.

Chaque étape est chronométrée (`Page::timings()`, type `LoadTimings`) et
exécutée sous `catch_unwind` (fonction `guarded`) : si une dépendance
paniquait malgré tout, la page est dégradée (message dans `Page::console()`)
au lieu de faire tomber le navigateur.

La rastérisation n'a pas lieu au chargement : l'appelant demande
`Page::render(pixmap, &RasterOptions)` pour la zone qu'il veut afficher.

## Crates et dépendances

| Crate | Rôle | Dépend de (crates internes) | Types principaux |
|---|---|---|---|
| `xbro-dom` | Arbre DOM en arène | — | `Document`, `NodeId`, `Node`, `NodeData`, `ElementData` |
| `xbro-html` | Décodage des octets, tokenizer, construction de l'arbre | dom | `parse_document`, `decode_html_bytes`, `Tokenizer`, `Token`, `MAX_DEPTH` |
| `xbro-css` | Tokenizer, parseur de feuilles, sélecteurs, media queries, couleurs | — | `Stylesheet`, `CssRule`, `Declaration`, `ComponentValue`, `Selector`, `Element` (trait), `MediaQueryList`, `Color` |
| `xbro-style` | Cascade, héritage, valeurs calculées, feuille UA, attributs de présentation | dom, css | `compute_styles`, `StyleContext`, `StyleMap`, `ComputedStyle`, `LengthPercentage` |
| `xbro-text` | Polices système, sélection, repli, shaping simple, glyphes | — | `FontSystem`, `FontId`, `FontQuery`, `ShapedRun`, `ShapedGlyph`, `FontMetrics`, `GlyphBitmap` |
| `xbro-net` | `http(s):`, `file:`, `data:`, `about:`, saisie de la barre d'adresse | — | `Fetcher`, `FetcherOptions`, `Response`, `NetError`, `parse_user_input`, `resolve` |
| `xbro-layout` | Arbre de boîtes ; bloc, inline, flottants, positionnement, flex, grille, tableaux | dom, css, style, text | `layout`, `LayoutContext`, `LayoutTree`, `LayoutBox`, `BoxKind`, `LineBox`, `Fragment`, `ReplacedSizes` (trait) |
| `xbro-paint` | Liste d'affichage, rastérisation, images | dom, css, style, text, layout | `DisplayList`, `DisplayItem`, `rasterize`, `RasterOptions`, `Image`, `ImageProvider` (trait) |
| `xbro-engine` | Chargement de page, sous-ressources, interactions | toutes les précédentes | `Engine`, `NavigationRequest`, `Page`, `ClickAction`, `PageImages`, `FindMatch` |
| `xbro` | Binaire : fenêtre (onglets, historique, barre d'adresse) et modes sans fenêtre | css, engine, net, paint, text | `Options`, `Mode`, `gui::browser::Browser`, `gui::app::App` |

Les dépendances vont toujours « vers le bas » du pipeline. Quand une étape
basse a besoin d'une information produite plus haut, elle déclare un trait
que l'étape haute implémente (inversion de dépendance, comme une interface
Java) :

- `xbro_css::selector::Element` : le DOM vu par le moteur de sélecteurs
  (implémenté par `xbro-style` pour les nœuds du `Document`) ;
- `xbro_layout::ReplacedSizes` : tailles naturelles des images, et
  `xbro_paint::ImageProvider` : pixels des images (tous deux implémentés par
  `xbro_engine::PageImages`, qui détient les images décodées).

## Décisions de conception transverses

### DOM en arène et `NodeId` stable

Tous les nœuds vivent dans un seul `Vec<Node>` possédé par le `Document` ;
un nœud est désigné par un `NodeId(usize)`, son index dans ce vecteur. Le
nœud 0 est toujours le nœud document. Les nœuds ne sont **jamais libérés** :
`detach` se contente de les retirer de la liste des enfants de leur parent,
donc un `NodeId` reste valide pendant toute la vie du document.

Pourquoi : en Rust, un graphe d'objets qui se référencent mutuellement
(parent ↔ enfants) se heurte au modèle de propriété (il faudrait des
`Rc<RefCell<…>>` et des `Weak`, coûteux et sujets aux cycles). Avec une
arène, les liens sont de simples entiers copiables, le DOM se partage en
lecture sans contrainte, et les étapes suivantes peuvent stocker leurs
résultats dans des tables annexes indexées par `NodeId` : `StyleMap::styles`
est un `Vec<Option<Arc<ComputedStyle>>>` indexé par `node.index()`, les
boîtes et fragments de texte de la mise en page portent le `NodeId` de leur
nœud, et `PageImages` associe les images aux `NodeId` (la liste d'affichage
les désigne ainsi : `DisplayItem::NodeImage { node, … }`).

### Unités : tout en px CSS, `LengthPercentage { px, percent }`

- Toutes les longueurs manipulées par le moteur sont des **px CSS** en `f32`.
  La conversion vers les pixels physiques (facteur HiDPI × zoom) n'a lieu
  qu'à la rastérisation (`RasterOptions::scale`).
- À la cascade, les longueurs absolues et relatives à la police ou au
  viewport (`em`, `rem`, `ch`, `ex`, `pt`, `vw`, `vh`…) sont résolues en px.
  Les **pourcentages**, qui dépendent du bloc conteneur, restent symboliques :
  `LengthPercentage { px, percent }` vaut `px + percent % × référence` et est
  résolu par la mise en page (`resolve(reference)`).
- Cette forme linéaire représente aussi `calc()` : `calc(100% - 2em)` à
  16 px devient `{ px: -32, percent: 100 }`. Le parseur de `xbro-style` replie
  toute expression `calc()` linéaire (sommes, produits par des nombres) en une
  combinaison de termes d'unités (`LinearLen`) dès l'analyse.
  `min()`/`max()`/`clamp()` qui mélangent pourcentages et longueurs sont
  résolus contre la largeur du viewport (approximation).
- `currentcolor` est résolu en couleur concrète à la cascade.
- Les valeurs sont assainies : NaN devient 0 et les longueurs sont bornées à
  ±10⁷ px, pour que l'arithmétique de mise en page reste finie.

### Coordonnées absolues et couches fixes

La mise en page produit des rectangles **absolus** dans le repère du
document (origine en haut à gauche du bloc conteneur initial). Exception :
les sous-arbres `position: fixed` sont en **coordonnées de viewport**
(`LayoutBox::is_fixed`). La liste d'affichage les encadre de
`DisplayItem::PushFixed` / `PopFixed` ; le rastériseur n'y applique pas le
défilement et repart d'une pile de découpage vide (un élément fixe n'est
pas découpé par l'`overflow` de ses ancêtres). Le test de position
(`LayoutTree::hit_test(point, scroll)`) et la recherche dans la page
(`FindMatch::fixed`) tiennent compte de ces couches ; `Page::fixed_backdrops`
donne les boîtes fixes qui peignent un fond (en-têtes, barres d'outils), sous
lesquelles le contenu défile, pour ne pas surligner des occurrences cachées.

### Liste d'affichage en px CSS, rastérisation de la seule zone visible

La `DisplayList` est une liste plate de commandes de dessin (rectangles,
bordures, dégradés, texte, images, découpages, groupes d'opacité…) en **px
CSS et coordonnées du document**, dans l'ordre de peinture. Elle ne dépend
ni du défilement, ni du zoom, ni de la densité de l'écran : la même liste est
rastérisée à n'importe quelle échelle et position, sans refaire style ni
mise en page.

`rasterize` (et `Page::render`) reçoit des `RasterOptions` :
`scroll` (point du document affiché en haut à gauche), `scale` (pixels
physiques par px CSS = HiDPI × zoom), `offset` et `height` (bande de lignes
de la cible à dessiner, le reste n'est pas touché). Seule cette bande est
dessinée et chaque commande est éliminée à bas coût si elle tombe hors du
découpage courant : défiler une page de 100 000 px ne coûte que la zone
visible. Dans la fenêtre, la page est rastérisée dans un calque de la taille
de la zone de page, mis en cache (clé : onglet, défilement, échelle, taille)
et recopié dans l'image de la fenêtre ; les éléments d'interface
(surlignages de recherche, barres de défilement, bulle d'état, barre
d'onglets) sont redessinés par-dessus sans re-rastériser la page.

### Threads

| Thread | Rôle |
|---|---|
| UI (thread principal) | boucle d'événements `winit`, modèle `Browser`, interactions avec les `Page`, rendu et présentation des images |
| `xbro-loader` (un par navigation) | appelle `Engine::load` (bloquant), puis poste `UserEvent::Loaded { tab_id, nav_id, page }` à la boucle d'événements via un `EventLoopProxy` |
| `xbro-fetch` (≤ 8 par page) | feuilles de style et `@import`, en threads *scoped* : le chargement attend la fin de tous (chaque requête est bornée par le délai du `Fetcher`) |
| `xbro-images` (≤ 8 par page) | images, en threads détachés : le chargement cesse d'attendre à l'échéance du budget, les retardataires finissent en arrière-plan et leur résultat est jeté |
| `xbro-svg` (éphémère, *scoped*) | chaque analyse ou rendu SVG, sur une pile de 64 Mio (`usvg`/`resvg` sont récursifs) ; l'appelant attend sa fin |

Règles de partage :

- `FontSystem` et `Fetcher` sont `Send + Sync` et partagés via `Arc` (un
  `Engine` se clone à bas coût). Les caches internes de `FontSystem` sont
  derrière des `RwLock`/`Mutex` tolérants à l'empoisonnement (un panic sur un
  autre thread ne les rend pas inutilisables).
- `Page` est `Send` mais pas partagée : construite sur le thread de
  chargement, elle est **déplacée** vers le thread UI. Toutes les interactions
  (survol, clic, saisie, redimensionnement) s'exécutent ensuite de façon
  synchrone sur le thread UI.
- Un résultat de chargement périmé (onglet fermé, autre navigation lancée
  depuis, chargement arrêté) est reconnu à son `nav_id` et ignoré.
- Le thread UI n'attend jamais le réseau. Il peut en revanche recalculer
  style et mise en page lors d'une interaction : c'est pourquoi les
  restylages `:hover` ont un budget (voir `xbro-engine`).

### Robustesse : ne jamais paniquer sur du contenu web

Aucune étape ne doit paniquer, quelle que soit l'entrée : HTML ou CSS
malformés, polices ou images corrompues, valeurs hostiles (`1e30px`, NaN),
pages énormes ou imbriquées à l'extrême. Les moyens :

- pas d'`unwrap`/`expect`/indexation non vérifiée sur des données venues du
  Web (`get()`, `checked_*`, `saturating_*`, `clamp` et assainissement des
  flottants) ;
- des **bornes explicites** sur tout ce qui peut croître (profondeur,
  nombre d'éléments, tailles d'images, travail des `var()`…), résumées dans
  la section [Budgets et limites](#budgets-et-limites) ;
- des algorithmes **itératifs ou à récursion bornée** : la profondeur du DOM
  est limitée à 256 éléments par le parseur, ce qui protège la cascade, la
  mise en page et la peinture, récursives par endroits ;
- `catch_unwind` en dernier recours aux frontières (étapes du pipeline,
  décodeurs d'images, thread de chargement, rastérisation, initialisation de
  la fenêtre) : un bug d'une dépendance dégrade la page au lieu de tuer le
  navigateur.

Des tests de robustesse (entrées aléatoires reproductibles, cas
pathologiques) couvrent chaque étape, jusqu'au pipeline complet (voir
[CONTRIBUTING.md](CONTRIBUTING.md), « Tests aléatoires »).

### Pas d'incrémentalité

Le style et la mise en page sont recalculés **en entier** à chaque
changement (survol, focus, saisie, redimensionnement, zoom) : pas d'arbre
« sale » partiel à maintenir. C'est un choix de simplicité rendu possible par
les performances : sur une page synthétique de ~1 Mo (plus de 14 000
éléments, test `perf` de `xbro-engine`), analyse + feuilles + cascade +
mise en page + liste d'affichage + une rastérisation 1024×768 prennent
≈ 0,3 s en `--release` sur la machine de développement, pour un budget
vérifié de 1,5 s. Des raccourcis évitent le travail inutile : une saisie ne
refait que la mise en page (sauf sélecteurs dépendant de la valeur), et un
survol ne restyle que si un sélecteur `:hover` peut changer de résultat.

## `xbro-dom` : l'arbre DOM

- `Document { nodes: Vec<Node>, quirks_mode, … }` ; `Node { parent:
  Option<NodeId>, children: Vec<NodeId>, data: NodeData }` ;
  `NodeData::{Document, Doctype, Element(ElementData), Text, Comment}` ;
  `ElementData { name, attrs }` avec un nom local en minuscules et les
  attributs dans l'ordre du source.
- Construction : `create_element`/`create_text`/…, `append_child`,
  `insert_before`, `append_text` (fusionne avec un texte précédent),
  `detach`.
- Lecture : `parent`, `children`, `ancestors`, `descendants`,
  `element_children`, `prev_sibling`/`next_sibling` (en O(1) grâce à un index
  des positions calculé paresseusement et invalidé par toute méthode `&mut`),
  `get_element_by_id`, `get_elements_by_tag_name`, `title`, `text_content`,
  `to_debug_string` (sortie de `--dump-dom`).
- Le DOM n'enregistre pas les espaces de noms : un élément SVG est reconnu à
  ses ancêtres.

## `xbro-html` : décodage et analyse HTML

- **Décodage** (`decode_html_bytes`) : BOM, puis `charset` du `Content-Type`
  HTTP, puis pré-analyse des 1024 premiers octets (`<meta charset>`,
  `<meta http-equiv=content-type>`), puis UTF-8 si les octets sont valides
  (une séquence tronquée à la toute fin est tolérée), sinon windows-1252.
  Décodage avec pertes (U+FFFD), jamais d'échec.
- **Tokenizer** (`tokenizer`) : machine à états WHATWG simplifiée. L'entrée
  est normalisée une fois (CRLF/CR → LF, NUL → U+FFFD). États data, RCDATA,
  RAWTEXT, script, PLAINTEXT, balises et attributs, commentaires, doctype
  (avec détection du mode quirks), sections CDATA en contenu étranger,
  références de caractères (table complète des entités HTML5, cas
  historiques sans point-virgule, corrections des références numériques).
  Le constructeur d'arbre pilote le modèle de contenu (`set_mode`) ; le
  tokenizer ne lit jamais au-delà du jeton rendu.
- **Construction d'arbre** (`tree_builder`) : algorithme WHATWG « scripting
  désactivé » : modes d'insertion (dont tableaux, `select`, `template`,
  `frameset`), pile des éléments ouverts, liste des éléments de formatage
  actifs avec l'algorithme d'adoption (adoption agency), foster parenting,
  contenu étranger SVG/MathML (casse des noms d'éléments et d'attributs SVG
  rétablie : `viewBox`, `linearGradient`…, balises auto-fermantes
  respectées). Le résultat contient toujours `html > head` et `html > body`.
- **Garde-fous** ajoutés à la spécification : profondeur du DOM ≤
  `MAX_DEPTH` = 256 (au-delà, les éléments sont rattachés à l'ancêtre le plus
  profond autorisé, et une passe finale aplatit ce que l'adoption aurait pu
  enfoncer), pile des éléments ouverts ≤ 512, ≤ 64 éléments de formatage
  actifs par section, nombre d'éléments recréés par la « reconstruction des
  éléments de formatage » proportionnel à la taille de l'entrée (pas
  d'amplification), retraitement d'un jeton borné.

## `xbro-css` : syntaxe, sélecteurs, media queries, couleurs

La crate ne connaît pas le DOM ni les propriétés : elle fournit les briques.

- **Tokenizer** : CSS Syntax 3 §4, linéaire (chaque étape consomme au moins
  un caractère avec une avance bornée). Les identifiants gardent leur casse,
  les échappements sont résolus.
- **Parseur** (`parse_stylesheet`, `parse_declaration_list`) : les jetons
  sont d'abord groupés en `ComponentValue` (blocs et fonctions, profondeur ≤
  64), puis règles et déclarations sont reconnues avec la récupération
  d'erreurs de la spécification. Résultat : `Stylesheet { rules:
  Vec<CssRule> }` avec `CssRule::{Style, Media, Supports, Import, Group}`.
  L'imbrication CSS (nesting) est aplatie en règles distinctes dont le
  sélecteur est résolu contre le parent. `@layer`, `@container` et
  `@document` deviennent des `Group` appliqués sans condition. `@supports` est
  évalué à l'analyse (`is_supported_property`). Les at-rules inconnues
  (`@font-face`, `@keyframes`, `@page`, `@namespace`…) sont ignorées. Les
  valeurs des déclarations restent des `ComponentValue` brutes : leur
  interprétation appartient à `xbro-style`.
  `Stylesheet::rewrite_urls` permet au moteur de rendre toutes les URL
  absolues par rapport à la feuille.
- **Sélecteurs** (`selector`) : Selectors 4 partiel. Combinateurs
  descendant, enfant, `+`, `~` ; type, `#id`, `.classe`, attributs (tous les
  opérateurs, drapeau `i`) ; `:root`, `:empty`, `:first/last/only-child`,
  `:*-of-type`, `:nth-child(an+b)` et variantes, `:link`, `:any-link`,
  `:hover`, `:active`, `:focus`, `:focus-within`, `:focus-visible`,
  `:checked`, `:disabled`, `:enabled`, `:not`, `:is`, `:where`, `:has`
  (formes descendante, enfant et sœurs), `:target`, `:lang`, `:dir`,
  `:defined` ; `::before`/`::after`. Les pseudo-classes reconnues mais non
  gérées (`:valid`, `:fullscreen`…) ne correspondent jamais (la règle reste
  valide), comme `:visited`. Spécificité calculée à l'analyse. Le moteur de
  correspondance passe par le trait `Element`. Bornes : ≤ 64 composés par
  sélecteur, imbrication des pseudo-classes fonctionnelles ≤ 16.
  `Selector::matches_with_cache` partage un `MatchCache` pendant une passe de
  cascade : les résultats de `:has()` y sont mémorisés par élément, ce qui
  évite de reparcourir les mêmes sous-arbres pour chaque règle.
- **Media queries** (`media`) : Media Queries 4 partiel, logique à trois
  valeurs (une caractéristique inconnue rend la requête fausse). Types
  `all`/`screen` (vrais) et `print` etc. (faux), `not`, `only`, `and`,
  `width`/`height` et leurs `min-`/`max-`, syntaxe d'intervalle
  (`(400px < width < 800px)`), `orientation`, `aspect-ratio`, `resolution`,
  `prefers-color-scheme`, `prefers-reduced-motion`, `hover`, `pointer`…
  évaluées contre un `MediaContext` (viewport, résolution, thème).
- **Couleurs** (`color`) : `Color { r, g, b, a }` 8 bits non prémultiplié ;
  couleurs nommées et système, hexadécimal, `rgb()`, `hsl()`, `hwb()`,
  `lab()`, `lch()`, `oklab()`, `oklch()`, `color()`, `color-mix()`,
  `light-dark()` (imbrication ≤ 8), et l'algorithme « legacy » des
  attributs HTML (`bgcolor="chucknorris"`). Les couleurs hors gamut sRGB
  sont écrêtées.

## `xbro-style` : cascade et valeurs calculées

Entrée : `compute_styles(&Document, &[Stylesheet], &StyleContext) ->
StyleMap`. Le `StyleContext` porte le `MediaContext` et les éléments survolé,
focalisé et actif (pour `:hover`, `:focus`, `:active`).

- **Indexation des règles** (`rules`) : les feuilles sont aplaties
  (`@media`/`@supports` évalués, groupes appliqués, `@import` ignorés car le
  moteur les a déjà insérés), les déclarations analysées une fois par règle,
  et chaque sélecteur est rangé dans un compartiment selon son composé sujet :
  id, première classe, balise, premier nom d'attribut, ou compartiment
  universel. Un élément ne teste que les règles de ses compartiments.
- **Filtre de Bloom des ancêtres** (`bloom`) : filtre à comptage des balises,
  ids et classes des ancêtres de l'élément courant ; un sélecteur exigeant
  un ancêtre absent du filtre est écarté sans exécuter le moteur de
  correspondance (optimisation classique des moteurs de navigateur).
- **Cascade** (`cascade`) : parcours du document en ordre d'arbre avec une
  pile explicite (pas de récursion). Pour chaque élément, les blocs retenus
  sont triés par rang puis spécificité puis ordre d'apparition. Rangs, du
  moins au plus prioritaire : UA normal < attributs de présentation
  (`bgcolor`, `width`, `<font color>`… : règles auteur de spécificité nulle
  placées avant toutes les feuilles) < auteur normal < attribut `style` <
  auteur `!important` < `style` `!important` < UA `!important`. Les
  sous-arbres `display: none` sont sautés ; les nœuds texte partagent l'`Arc`
  du style de leur parent. Mots-clés `inherit`, `initial`, `unset`, `revert`
  (retour à la valeur UA, il n'y a pas d'origine utilisateur).
- **Calcul des valeurs** (`compute`, `properties`, `values`) : chaque
  propriété longue a un analyseur ; les raccourcis (`margin`, `border`,
  `font`, `background`, `flex`, `grid-template`, `list-style`…) sont
  développés. `font-family`, `font-size`, `line-height` et `color` sont
  calculées d'abord, car les autres en dépendent (`em`, `ch`,
  `currentcolor`). Les propriétés logiques (`margin-inline-start`…) sont
  converties en physiques pour un texte horizontal de gauche à droite.
- **Propriétés personnalisées** (`vars`) : héritées par partage d'`Arc` ; une
  table qui change ne stocke que ses propres entrées et pointe vers celle du
  parent (aplatie de temps en temps). Les valeurs stockées sont déjà
  substituées. Les cycles de dépendances (repliements compris) sont détectés
  par composantes fortement connexes et invalident leurs propriétés. Tout le
  travail est mémoïsé et décompté d'un **budget par document** (32 millions
  d'unités) : une page hostile qui l'épuise voit ses `var()` échouer (valeur
  `unset`) au lieu de prendre un temps proportionnel à éléments × taille des
  valeurs. Une substitution produit au plus 100 000 valeurs, avec une
  imbrication ≤ 64.
- **Feuille UA** (`ua`) : d'après la section « Rendering » du standard HTML
  et les valeurs de Chrome, plus une feuille propre au mode quirks.
- **Sortie** : `StyleMap { styles, before, after }` ; `styles[i]` est `Some`
  pour chaque élément qui génère des boîtes et pour les textes de ces
  éléments. `ComputedStyle` regroupe `display`, `position` et décalages,
  `float`/`clear`, modèle de boîte (`width`… en `LengthPercentageAuto`,
  `MaxSize`, `Sides<…>` pour marges, remplissages et bordures, rayons),
  `overflow`, `opacity`, `visibility`, arrière-plan (une couche : couleur,
  `Url` ou `LinearGradient`, `repeat`, `size`, `position`), police
  (`font_family: Arc<Vec<String>>`, taille en px, graisse, style), texte
  (`line-height`, `text-align`, `white-space`, `overflow-wrap`,
  `word-break`, décorations…), listes, tableaux, flex, grille
  (`GridTemplate` partagés), `content`, `cursor`.

Approximations documentées dans `lib.rs` : tailles intrinsèques
(`min-content`, `fit-content`…) calculées en `auto` hors des pistes de
grille, lignes de grille nommées → `auto`, `subgrid`/`masonry` → `none`,
dégradés radiaux et coniques réduits à leur première couleur, positions de
couleurs en longueur réparties comme si elles étaient omises,
`word-break: break-word` traité comme `overflow-wrap: break-word`.

## `xbro-text` : polices, shaping et glyphes

- **`FontSystem::new`** découvre les polices système avec `fontdb` (dossiers
  standard et configuration fontconfig, lue en Rust) et celles de
  `XBRO_FONT_DIRS`. Les fichiers ne sont analysés (par `ab_glyph`) qu'à leur
  première utilisation. `FontSystem::from_fonts` construit un système
  déterministe à partir d'octets (utilisé par les tests).
- **Sélection** (`select(&FontQuery)`) : appariement CSS Fonts 4 §5.2
  (étirement, puis style, puis graisse la plus proche) sur la première
  famille disponible. Les familles génériques (`sans-serif`, `serif`,
  `monospace`, `system-ui`…) pointent vers des polices installées
  (DejaVu, Liberation, Noto…) ; les polices connues mais absentes (Arial,
  Helvetica, Times New Roman, Segoe UI, Roboto…) vers un équivalent
  métrique ou générique. Les polices bitmap (emoji couleur) sont écartées.
  Résultats mis en cache.
- **Synthèse** : si la famille n'a pas de face italique ou grasse, le
  `FontId` retourné porte des bits de synthèse (deux bits de poids fort) :
  oblique par cisaillement d'environ 14°, gras par dilatation horizontale
  (taille / 24) avec avances élargies d'autant.
- **Shaping simple** (`shape`, `measure`) : de gauche à droite, un glyphe par
  caractère (correspondance `cmap`), avances, crénage de la table `kern`
  (pas de GPOS/GSUB, donc ni ligatures ni écritures complexes),
  `letter-spacing` et `word-spacing`. Un caractère absent de la police est
  pris dans une police de **repli** qui le contient (recherche dans la table
  `cmap` brute de chaque face, sans copie, au plus 200 nouvelles faces
  examinées par recherche ; résultat mis en cache par caractère).
- **Métriques** (`metrics`) : ascendante, descendante, interligne, hauteur
  d'x, position et épaisseur du soulignement (table `post`), toujours finies
  et positives.
- **Rastérisation** (`FontSystem::rasterize`) : masques de couverture 8 bits
  via `ab_glyph`, dans un cache borné (20 000 entrées, 64 Mio, glyphes ≤
  2048 px).
- **Mode dégradé** : sans aucune police, le texte est mesuré à 0,5 em par
  caractère et ne se dessine pas ; tailles bornées à 10⁶ px.

`FontSystem` est `Send + Sync` et partagé par l'interface (texte de la barre
d'outils) et les threads de chargement.

## `xbro-net` : chargement des ressources

- **`Fetcher`** (`Send + Sync`) aiguille selon le schéma :
  - `http:`/`https:` : `ureq` (HTTP/1.1) avec `rustls` et les racines Mozilla
    (`webpki-roots`), ou le fichier PEM de `SSL_CERT_FILE`. Les
    **redirections sont suivies par xbro** (10 au plus) plutôt que par `ureq`,
    pour décider du proxy à chaque saut, n'accepter que des cibles HTTP(S) et
    conserver le fragment. Délai global de 30 s par chargement (redirections
    et corps compris), corps limité à 50 Mio (après décompression : pas de
    bombe gzip), cookies conservés en mémoire par l'agent `ureq`, gzip et
    brotli. Les statuts 4xx/5xx sont des réponses normales (la page d'erreur
    du site s'affiche). En-têtes `User-Agent` (`Mozilla/5.0 (X11; Linux
    x86_64) xbro/0.1.0`), `Accept`, `Accept-Language` dérivé de la locale.
  - **Proxys** (`proxy`) : lus dans l'environnement par xbro (et non par
    `ureq`, qui ne gère qu'un proxy et un `NO_PROXY` simplifié) : un proxy par
    schéma, `NO_PROXY` avec domaines, IP, plages CIDR et ports, hôtes locaux
    toujours en direct.
  - `file:` : fichier (type MIME d'après l'extension, sinon reniflé) ou
    listing HTML d'un dossier (10 000 entrées au plus).
  - `data:` : décodé selon le standard Fetch (base64 ou pourcentages).
  - `about:home` et `about:help` : pages HTML embarquées dans le binaire
    (`src/about/*.html`, `include_str!`) ; `about:blank` : page vide.
  - autre schéma : `NetError::UnsupportedScheme`.
- **`Response { url, status, content_type, charset, body }`** et
  **`NetError::{UnsupportedScheme, Io, TooManyRedirects, TooLarge, Invalid,
  NotFound}`** (messages lisibles, affichés dans les pages d'erreur).
- Fonctions libres : `parse_user_input` (interprétation de la barre
  d'adresse : URL, chemin local, nom d'hôte ou recherche DuckDuckGo HTML),
  `resolve` (URL relative), `decode_text`, `encode_form`.

## `xbro-layout` : mise en page

Entrée : `layout(&LayoutContext { document, styles, fonts, replaced,
viewport, focused })` ; sortie : `LayoutTree { root: Option<LayoutBox>,
viewport, document_size }`.

1. **Arbre de boîtes** (`box_tree`) : le DOM et la `StyleMap` deviennent des
   `BoxNode` : blocs anonymes autour des suites de contenu en ligne, découpage
   d'un inline qui contient un bloc, `display: contents`, contenu généré,
   réparations des tableaux (`table::fixup`), éléments remplacés et contrôles
   de formulaire (boîtes atomiques ; on n'entre jamais dans un `<svg>`).
   L'`overflow` de la racine (ou de `<body>`) est propagé au viewport.
2. **Mise en page** : une passe sur l'arbre de boîtes, avec des caches valables
   pour la passe (`LayoutCx` : polices, largeurs de mots, tailles
   intrinsèques min/max-content mémorisées par boîte, de sorte que des boîtes
   « shrink-to-fit » imbriquées ne sont mesurées qu'une fois).
   - **Bloc** (`block`) : fusion des marges (avec le premier/dernier enfant,
     boîtes vides traversées), contextes de formatage de bloc (BFC) pour la
     racine, les flottants, les absolus, les inline-blocks, `overflow ≠
     visible`, flex, tableaux et éléments remplacés ; hauteur auto qui
     englobe les flottants d'un BFC.
   - **Inline** (`inline`) : (1) collecte des mots, espaces, coupures
     forcées, débuts et fins de boîtes en ligne, éléments atomiques,
     flottants et absolus, avec traitement de `white-space` et
     `text-transform` et mesure de chaque mot ; (2) coupure gloutonne aux
     occasions de coupure (espaces, après un tiret, `<wbr>`, U+200B, U+00AD,
     autour des idéogrammes CJK et des éléments atomiques ; coupure dans un mot
     selon `overflow-wrap`/`word-break`) ; (3) construction des lignes :
     espaces de fin retirés, `text-align` (dont `justify`), `text-indent`,
     hauteur de ligne avec demi-interlignage et `vertical-align`, production
     des `Fragment`. Les lignes voisines d'un flottant sont raccourcies.
   - **Flottants** (`float`) : un `FloatContext` par BFC, en coordonnées
     absolues ; `clear`.
   - **Positionnement** (`position`) : `relative` décale le sous-arbre déjà
     placé ; `absolute` est enregistré à sa position statique puis placé quand
     la boîte de remplissage de son bloc conteneur est connue (il devient
     enfant de ce bloc) ; `fixed` idem avec le viewport, en coordonnées de
     viewport ; `sticky` se comporte comme `relative` sans décalage.
   - **Flex** (`flex`) : Flexbox 1 §9 simplifié : les quatre directions,
     `wrap`, `grow`/`shrink`/`basis`, `justify-content`, `align-items/self/
     content`, `gap`, `order`, marges `auto`, alignement sur la ligne de base
     en rangée seulement.
   - **Grille** (`grid`) : grille explicite (`px`, `%`, `fr`, `auto`,
     `min-content`, `max-content`, `minmax()`, `fit-content()`,
     `repeat(auto-fill | auto-fit, …)`), placement en rangées par numéros de
     lignes et `span`, dimensionnement des pistes selon CSS Grid 1 §11
     (contributions des éléments, éléments couvrant plusieurs pistes par ordre
     croissant de portée, pistes flexibles), puis alignement. Non gérés :
     lignes nommées et zones, `grid-auto-flow: column`/`dense`,
     `grid-auto-rows/columns`.
   - **Tableaux** (`table`) : largeur automatique et fixe (`table-layout`),
     `colspan`/`rowspan`, `<col>`/`<colgroup>`, bordures séparées
     (`border-spacing`) et fusionnées (résolues entre cellules et table
     seulement), légendes en haut ou en bas, alignement vertical des cellules.
   - **Éléments remplacés** (`replaced`) : tailles naturelles via le trait
     `ReplacedSizes` fourni par le moteur, attributs `width`/`height`,
     tailles par défaut (300×150 pour `<svg>`, `<iframe>`, `<video>`,
     `<canvas>`…), texte alternatif d'une image absente mis en page comme un
     inline-block, contrôles de formulaire avec leur texte ; un champ focalisé
     est « défilé » pour montrer la fin de sa valeur.
   - **Listes** (`list`), **`fieldset`/`legend`** (`fieldset`), tailles
     intrinsèques (`intrinsic`).
3. **Requêtes** (`query`, `dump`) : `hit_test(point, scroll)` (dans l'ordre
   de peinture), `node_rect` (ancres `#id`), `text_fragments` (recherche),
   `document_size`, `dump` (sortie de `--dump-layout`) et `outline`
   (comparaisons dans les tests).

Le résultat : `LayoutBox { node, pseudo, style, kind: BoxKind, content,
padding, border, margin, children, lines, marker, border_top_gap }` ;
`BoxKind::{Block, Flex, Table, TableRowGroup, TableRow, TableCell, Image,
Placeholder, Control(ControlKind)}` ; `LineBox { rect, baseline, fragments }`
avec `Fragment::{Text(TextFragment), InlineBox(InlineBoxFragment),
Atomic(LayoutBox)}`. Un `TextFragment` porte ses glyphes déjà positionnés
(`Vec<ShapedGlyph>`), ce qui évite tout nouveau shaping à la peinture.

## `xbro-paint` : liste d'affichage et rastérisation

- **Construction** (`build_display_list_full(tree, fonts, images)`) : ordre
  de peinture de CSS 2.1 annexe E, simplifié. Contextes d'empilement : la
  racine, les positionnés avec `z-index` ≠ `auto`, les éléments flex avec
  `z-index`, `fixed` et `sticky`, `opacity < 1`. Dans chacun : fond de la
  racine, contextes à `z-index` négatif, fonds et bordures des blocs,
  flottants (peints atomiquement), contenu en ligne (fonds des boîtes en
  ligne, texte et décorations, éléments atomiques), positionnés `z-index:
  auto`/`0`, contextes à `z-index` positif. `overflow ≠ visible` produit un
  `PushClip` (boîte de remplissage, coins arrondis compris), `opacity` un
  groupe `PushOpacity`. `DisplayList::background` est la couleur du canevas
  (propagée depuis la racine ou `<body>`).
- **Commandes** (`DisplayItem`) : `Rect`, `Border` (quatre côtés, rayons),
  `LinearGradient`, `Text(GlyphRun)`, `TextDecoration`, `NodeImage`,
  `BackgroundImage`, `DeferredBackgroundImage` (géométrie résolue à la
  rastérisation quand la taille de l'image n'était pas connue), `UrlImage`,
  `Push/PopClip`, `Push/PopOpacity`, `Push/PopFixed`, `Ellipse`, `Polyline`,
  `Polygon` (marqueurs de liste, cases à cocher, flèches de `<select>`).
- **Rastérisation** (`rasterize`) : transformation `pixel = (doc − scroll) ×
  scale + offset` (sans `scroll` dans les sections fixes), élimination contre
  le découpage courant, rectangles alignés sur les pixels, formes arrondies
  anti-crénelées, bordures par côté (jonctions diagonales ; styles `dashed`,
  `dotted`, `double`, `groove`, `ridge`, `inset`, `outset`), texte à partir
  des masques de glyphes mis en cache et mélangés dans une boucle dédiée,
  images avec filtrage bilinéaire (réduction préalable par moyenne pour les
  fortes réductions, sans moiré), SVG rendu par `resvg` à la taille de
  destination (cache borné ; au-delà de 1024 × 1024 px seule la partie visible
  est rendue, par blocs de 256 px, et un SVG à filtres est plafonné à
  512 × 512 px puis agrandi), découpages arrondis par masques construits à la
  demande, groupes d'opacité dans des calques transparents composés sur leur
  seule zone modifiée.
- **Images** (`image`) : `Image::Raster(Arc<Pixmap>)` (prémultiplié) ou
  `Image::Svg(Arc<usvg::Tree>)` ; `Image::decode` vérifie les dimensions dans
  l'en-tête **avant** d'allouer (≤ 16 384 px de côté, ≤ 100 mégapixels),
  applique l'orientation EXIF, ne garde que la première image d'un GIF/WebP
  animé, et s'exécute sous `catch_unwind`. Les SVG passent par une
  **pré-analyse structurelle** linéaire (`svg_scan`) qui rejette une
  imbrication > 256, plus de 16 références de découpage/masque chaînées et
  des entités DTD dont l'expansion dépasserait 1 Mio, car `usvg` est récursif
  (un débordement de pile ne se rattrape pas) ; analyse et rendu se font
  ensuite sur un thread à grande pile. `resvg` est compilé sans `text` : les
  `<text>` SVG ne sont pas rendus.
- `draw_text` : aide au dessin de texte utilisée par l'interface du
  navigateur.

## `xbro-engine` : la page

- **`Engine { fonts: Arc<FontSystem>, fetcher: Arc<Fetcher> }`** : services
  partagés, clonables. `load(&NavigationRequest, viewport) -> Page` ne
  renvoie jamais d'erreur (une erreur devient une page d'erreur dont `url()`
  reste l'URL demandée) ; `load_html(html, url, viewport)` part d'une chaîne.
  `NavigationRequest::{Get(Url), Post { url, fields }}`.
- **Sous-ressources** (`stylesheets`, `images`, `fetch`) : feuilles et
  `@import` (profondeur ≤ 4, ≤ 64 imports, ≤ 256 feuilles liées ; un import
  qui reviendrait vers une feuille de la chaîne est ignoré) niveau par
  niveau ; images avec un budget par page (400 URL, 20 s, 512 Mio de pixels
  décodés, comptés depuis l'en-tête avant décodage) ; ce qui dépasse est
  ignoré et signalé dans `Page::console()` (≤ 1000 messages). `srcset` et
  `<picture>` : choix des candidats 1x, ou de la plus petite image au moins
  aussi large que l'emplacement (plafonnée à 1200 px). Politique d'origine :
  seuls les documents `file:` peuvent charger des sous-ressources `file:`.
- **SVG en ligne** (`svg`) : le sous-arbre `<svg>` du DOM est resérialisé en
  document SVG autonome (espaces de noms déclarés, `<script>` et
  `<foreignObject>` retirés, couleur calculée de l'élément passée pour
  `currentColor`, éléments référencés par `<use href="#id">` hors du `<svg>`
  recopiés dans un `<defs>`), puis rendu comme une image ; les arbres
  identiques partagent un seul document, et les SVG distincts sont bornés à
  8 Mio par page.
- **`Page`** contient l'URL finale, le `Document`, les feuilles, la
  `StyleMap`, le `LayoutTree`, la `DisplayList`, les images, le focus et le
  survol, la console et les chronométrages. API d'interaction :
  `render`, `set_viewport` (restyle : media queries et unités de viewport),
  `node_at`, `link_at`, `cursor_at`, `set_hover`, `click` → `ClickAction::
  {None, Navigate(NavigationRequest), ScrollTo(y), Repaint}`, `focus_next`,
  `activate_focused`, `insert_text`, `backspace`, `submit_focused`, `blur`,
  `caret_rect`, `find_matches`, `fragment_offset`, `fixed_backdrops`,
  `meta_refresh`.
- **Interactions** : l'état des contrôles est écrit dans le DOM, là où la mise
  en page le lit (`value`, `checked`, `selected`, texte du `<textarea>`,
  `details[open]`) ; puis restyle complet, ou seulement nouvelle mise en
  page pour une saisie simple ; puis reconstruction de la liste d'affichage
  (anneau de focus et curseur de texte ajoutés à la fin). Formulaires
  (`forms`) : soumission GET ou POST `application/x-www-form-urlencoded` en
  UTF-8, soumission implicite par `Entrée`, bouton de réinitialisation
  (valeurs initiales conservées dans `FormDefaults`), `<label>`, `<select>`
  qui passe à l'option suivante, `<details>`/`<summary>`, cartes d'images
  (`image_map`). La recherche (`find`) est insensible à la casse et peut
  franchir les retours à la ligne d'un même nœud texte.
- **Budget `:hover`** (`dynamic`, `page`) : une analyse des feuilles
  (`DynamicRules`) garde la partie statique (balise, ids, classes, noms
  d'attributs) de chaque composé contenant `:hover` ; quand l'élément survolé
  change, le restyle n'a lieu que si un élément entrant ou sortant de la
  chaîne survolée peut correspondre à l'un d'eux. Un restyle coûtant autant
  que la cascade + mise en page + liste initiales, les effets `:hover` sont
  désactivés pour une page dont ce coût dépassait 250 ms au chargement (1 s en
  debug), ou dont un restyle a pris plus du double.
- **Pages générées** (`error_pages`) : erreurs réseau et HTTP, texte brut,
  image seule, contenu non affichable ; documents autonomes où tout texte
  externe est échappé.

## `xbro` : l'application

- **`main.rs`** : `parse_args` → `Options { mode, url, width, height, scale }`
  avec `Mode::{Gui, Screenshot { output }, DumpDom, DumpLayout, Help,
  Version}` ; `resolve_input` transforme un chemin existant (relatif au
  dossier courant) en URL `file://`, le reste passe par `parse_user_input`.
- **`headless.rs`** : charge la page avec `Engine::load`, puis écrit un PNG
  (`Page::render` dans un `Pixmap`, taille bornée à 32 768 px de côté et
  2²⁸ pixels) ou imprime `Document::to_debug_string` / `LayoutTree::dump`.
- **`gui/`** : l'interface graphique, organisée pour que presque tout soit
  testable sans fenêtre :
  - `browser.rs` : **`Browser`**, le modèle (onglets, navigation et
    chargements, défilement, zoom, recherche, focus, bilan des zones à
    redessiner), indépendant de `winit` ; les chargements passent par un
    rappel `Loader` et reviennent par `Browser::on_loaded` ;
  - `input.rs` : décodage pur des touches en commandes (`Command`,
    `PageCommand`, `FieldCommand`, `EditAction`), avec repli sur la position
    physique des touches ; `events.rs` : clavier et souris appliqués au
    `Browser` ;
  - `app.rs` : la glu `winit` (`ApplicationHandler`) : fenêtre, surface
    `softbuffer`, traduction des événements, cadence de rafraîchissement
    (`ControlFlow::Wait`, réveils pour l'animation de chargement et les
    `<meta refresh>`) ;
  - `loader.rs` : un thread `xbro-loader` par navigation, panic → page
    d'erreur ;
  - `render.rs` (composition d'une image : calque de page en cache,
    surcouches, barre d'onglets, barre d'outils, barre de recherche),
    `chrome.rs` (géométrie et test de position de l'interface, en px
    logiques), `paint.rs` (primitives de dessin et icônes vectorielles),
    `viewport.rs` (niveaux de zoom, conversions px CSS ↔ pixels physiques,
    barres de défilement), `tabs.rs`, `history.rs` (100 entrées par onglet,
    position de défilement mémorisée), `find.rs`, `line_edit.rs` (modèle
    d'édition sur une ligne), `address_bar.rs`, `clipboard.rs` ;
  - `stress_tests.rs` : test « singe » qui envoie de longues séquences
    aléatoires de touches, clics, redimensionnements, chargements et tics
    d'horloge au `Browser` et au moteur de rendu.
- **Zoom** : la zone de page fait `largeur / (HiDPI × zoom)` px CSS ; un
  changement de zoom refait la mise en page à cette largeur
  (`Page::set_viewport`) et rastérise à l'échelle `HiDPI × zoom`, comme les
  navigateurs. Niveaux : 50, 67, 80, 90, 100, 110, 125, 150, 175, 200, 250 et
  300 %.
- **Échec d'ouverture de la fenêtre** : `winit` peut paniquer au lieu de
  renvoyer une erreur (bibliothèque `libxkbcommon-x11` absente, par exemple) ;
  la création de la boucle d'événements est donc encadrée par
  `catch_unwind` et transformée en message d'erreur.

## Budgets et limites

| Domaine | Limite | Où |
|---|---|---|
| DOM | profondeur ≤ 256 éléments ; pile des éléments ouverts ≤ 512 ; ≤ 64 éléments de formatage actifs par section | `xbro-html` (`tree_builder`) |
| CSS | imbrication des blocs ≤ 64 ; ≤ 64 composés par sélecteur ; pseudo-classes fonctionnelles imbriquées ≤ 16 ; fonctions de couleur imbriquées ≤ 8 | `xbro-css` |
| `var()` | budget de 32 millions d'unités de travail par document ; ≤ 100 000 valeurs par substitution ; imbrication ≤ 64 ; cycles invalidés | `xbro-style` (`vars`) |
| Valeurs | longueurs bornées à ±10⁷ px ; arbres `calc()` ≤ 256 nœuds ; `content` ≤ 256 éléments et 64 Kio ; dégradés ≤ 256 arrêts ; grilles ≤ 1000 pistes, lignes ≤ 10 000, budget de pistes par document | `xbro-style` |
| Polices | tailles ≤ 10⁶ px ; ≤ 200 faces examinées par recherche de repli ; cache de glyphes ≤ 20 000 entrées / 64 Mio ; glyphe ≤ 2048 px | `xbro-text` |
| Réseau | 30 s par chargement ; 10 redirections ; corps ≤ 50 Mio ; fichier `SSL_CERT_FILE` ≤ 16 Mio ; listing ≤ 10 000 entrées | `xbro-net` |
| Sous-ressources | 8 chargements simultanés ; `@import` profondeur ≤ 4 et ≤ 64 ; ≤ 256 feuilles liées ; images : 400 URL, 20 s, 512 Mio décodés ; SVG en ligne ≤ 8 Mio ; console ≤ 1000 messages | `xbro-engine` |
| Images | ≤ 16 384 px de côté et ≤ 100 mégapixels ; SVG : imbrication ≤ 256, ≤ 16 références chaînées, entités ≤ 1 Mio, rendu ≤ 4096×4096, pile de 64 Mio | `xbro-paint` (`image`) |
| Rastérisation | échelle ≤ 64 ; calques d'opacité ≤ 256 Mio et 64 simultanés (au-delà, groupe dessiné sans sa propre opacité) | `xbro-paint` (`raster`) |
| Interactions | restyle `:hover` ≤ 250 ms (sinon désactivé) ; recherche ≤ 10 000 occurrences | `xbro-engine` |
| Interface | captures ≤ 32 768 px de côté et 2²⁸ pixels ; fenêtre ≤ 2²⁷ pixels dessinés ; historique ≤ 100 entrées par onglet ; `<meta refresh>` ≤ 30 s (≥ 1 s pour un rechargement de la page elle-même) | `xbro` |

## Pour commencer à lire le code

- Suivre un chargement : `xbro_engine::Engine::load` → `loader::load` →
  `loader::build_page`, puis `Page::relayout_timed`.
- Une propriété CSS de bout en bout : son analyse dans
  `crates/xbro-style/src/properties/` (table `properties/table.rs`), son
  champ dans `ComputedStyle` (`computed.rs`), son usage dans `xbro-layout` ou
  `xbro-paint/src/display_list/builder.rs`.
- Une touche du clavier : `gui/input.rs` (décodage) → `gui/events.rs`
  (`Browser::key`) → `gui/browser.rs`.
- Pour observer : `--dump-layout` pour l'arbre de boîtes, `--screenshot`
  pour le rendu, `Page::console()` (affiché sur la sortie d'erreur en mode
  sans fenêtre) pour les ressources en échec.
