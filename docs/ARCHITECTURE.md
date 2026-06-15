# Architecture — fouduvolant v2

Refonte de fouduvolant. Objectifs : code Rust simplifié, front MVU, write model
event-sourced.

## Stack

| Couche | Choix |
|--------|-------|
| Backend | Rust, CQRS + Event Sourcing (`cqrs-es` 0.5) |
| Event store | PostgreSQL (`postgres-es` 0.5) |
| Front | Elm (MVU pur, interop via ports) |

## Write model — agrégats

Deux agrégats, deux frontières de cohérence distinctes :

### `Tournament` (`backend/crates/domain/src/tournament.rs`)
Lifecycle setup. Phases (sens unique) : `NotCreated → Draft → PoolPhase → BracketPhase → Done`.

- Commands : `Create`, `RegisterTeam`, `RemoveTeam`, `GeneratePools`, `ConfigureCourts`, `StartPoolPhase`, `StartBracketPhase`
- Events : `Created`, `TeamRegistered`, `TeamRemoved`, `PoolsGenerated`, `CourtsConfigured`, `PoolPhaseStarted`, `BracketPhaseStarted`
- Invariants : changements setup uniquement en `Draft` ; pas d'équipe dupliquée ;
  poules non vides, équipes enregistrées, une équipe dans une seule poule ;
  démarrage pool exige poules + terrains.

### `Match` (`backend/crates/domain/src/matches.rs`)
Un agrégat par match. Hot path d'écriture, isolé du Tournament.
Lifecycle : `NotStarted → Scheduled → InProgress → Completed`.

- Commands : `Schedule{tournament_id,…}`, `Start{court}`, `RecordSet`
- Events : `Scheduled{tournament_id,…}`, `MatchStarted{court}`, `SetRecorded`, `Completed`
- `tournament_id` porté par `Schedule` → permet de scoper le dispatch par tournoi.
- Format **paramétrable** BO1 (poule) / BO3 (bracket) — porté par `Schedule`.
- `Start{court}` = mise sur terrain + début de jeu (status InProgress). Le terrain
  ici = où le match se **joue** ; les hints de scheduling (terrain suggéré /
  épinglé) sont côté lecture, hors agrégat.
- `RecordSet` exige InProgress. Auto-complétion : émet `Completed` dès que le
  nb de sets gagnants requis est atteint.

### Value objects (`backend/crates/domain/src/score.rs`)
- `MatchFormat` : `BestOf1` / `BestOf3`.
- `SetScore` : score de set **validé à la construction** (règle badminton : 21,
  win-by-2, cap 30 → 30-29 valide). Un `SetScore` existant est toujours valide.

### Identifiants (`backend/crates/domain/src/ids.rs`)
Newtypes sur `Uuid` : `TournamentId`, `TeamId`, `MatchId`, `PoolId`, `CourtId`.
Typage fort — impossible de confondre deux ids.

## Persistence (`backend/crates/app`)

Façade `App` = event store PostgreSQL via `postgres-es`. Le framework cqrs-es est
générique **par agrégat** → un `PostgresCqrs` par agrégat (Tournament + Match),
partageant un seul `Pool<Postgres>`.

- `App::connect(url)` / `from_pool(pool)` ; `run_migrations()` applique
  `db/init.sql` (tables `events` + `snapshots`, idempotent) via `sqlx::raw_sql`.
- `App::tournament(id, cmd)` / `App::match_cmd(id, cmd)` → `execute(id.to_string(), cmd)`.
- Queries (read models persistés) = **vec vide pour l'instant** (différé).
- Schéma : `events(aggregate_type, aggregate_id, sequence, event_type,
  event_version, payload json, metadata json)`, PK composite.

### Dev / test
Postgres 16 local (127.0.0.1:5432). DB+user dédiés :
`postgresql://fouduvolant:fouduvolant@localhost:5432/fouduvolant`.
Test d'intégration : `crates/app/tests/integration.rs` (`#[ignore]`), lancé via
`DATABASE_URL=… cargo test -p app -- --ignored`. ✅ vert (flow tournoi + match
complet, réhydratation vérifiée).

## HTTP API (`backend/crates/web`)

Binaire axum `fouduvolant` — couche fine sur `app`. CORS permissif (dev),
TraceLayer. State = `Arc<App>`. Ids des nouveaux agrégats générés côté serveur,
renvoyés dans la réponse.

Endpoints :
| Méthode | Route | Effet |
|---|---|---|
| GET  | `/tournaments` | liste (id, name, phase) |
| POST | `/tournaments` | créer → `{id}` |
| GET  | `/tournaments/{id}` | `TournamentView` (404 si absent) |
| POST | `/tournaments/{id}/teams` | enregistrer équipe → `{id}` |
| DELETE | `/tournaments/{id}/teams/{team_id}` | retirer |
| POST | `/tournaments/{id}/pools` | générer poules (PoolId serveur) |
| POST | `/tournaments/{id}/courts` | `{count}` → `{courts:[…]}` |
| POST | `/tournaments/{id}/start-pools` \| `/start-bracket` | transition phase |
| POST | `/tournaments/{id}/matches` | planifier match → `{id}` |
| POST | `/tournaments/{id}/dispatch` | process manager → `{started:[…]}` |
| GET  | `/tournaments/{id}/board` | `BoardView` (court plans + matchs) |
| POST | `/matches/{id}/start` | `{court_id}` |
| POST | `/matches/{id}/sets` | `{a,b}` |

Erreurs : `AggregateError::UserError` → 422, `AggregateConflict` → 409, reste →
500 ; `AppError` → 500 ; not found → 404. JSON `{"error": …}`.

Lancer : `PORT=3939 DATABASE_URL=… cargo run -p web` (défaut : DB dev locale,
port 3000). Migrations appliquées au démarrage.

## Frontend (`frontend/`, Elm 0.19.1 MVU)

SPA `Browser.element`, `src/Main.elm`. Deux écrans : liste tournois (+ création) et
tournoi sélectionné (config + plateau live). Le plateau **poll toutes les 3 s**
(`Time.every`) et après chaque mutation → reflète le backend event-sourced.

- Décodeurs alignés sur l'API (`TournamentView`, `BoardView`, `CourtPlan`,
  `MatchView`). Mutations via `Http.expectWhatever` (réponses 204), création via
  `expectJson` (récupère l'`id`).
- **▶ Démarrer** = `POST /matches/{id}/start {court_id}` (pas de machinerie
  `manual_court` côté back pour le MVP). **⟳ Dispatch auto** = `POST …/dispatch`.
- `apiBase` injecté par `index.html` (défaut `http://localhost:3000`, override
  `?api=`).

Build : `cd frontend && npm install && ./node_modules/.bin/elm make src/Main.elm --output=elm.js`.
Servir : n'importe quel statique (ex. `python3 -m http.server 8080`) ; CORS
permissif côté backend autorise l'origine distincte.

Lancer la stack complète :
1. `cd backend && cargo run -p web` (API :3000, migrations auto)
2. `cd frontend && python3 -m http.server 8080` → ouvrir `localhost:8080`

## Classements poule (`backend/crates/domain/src/standings.rs`)

`pool_standings(teams, results)` = pur, tiebreakers BWF dans l'ordre : **wins →
H2H (parmi les équipes à égalité de wins) → diff de points → points marqués →
team id**. H2H appliqué *avant* la diff, uniquement dans un groupe à égalité de
wins (comportement de l'original). Équipes sans match incluses (0 joué).

`App::standings(tid)` (replay) : enrichit `MatchView` (winner + points cumulés
par côté via projection), groupe par poule, classe. Endpoint
`GET /tournaments/{id}/standings` → `[{pool_id, name, rows:[…rank,wins,diff…]}]`.
Front : tables de classement par poule, rafraîchies au poll + après score.

## Read models (projections persistés Query — à venir)
Construits par replay des events, hors agrégats :
- `PoolStandings` — classement + tiebreakers BWF (wins → H2H → diff pts)
- `CourtBoard` / `Schedule` — planning par terrain + prévisionnel
- `BracketTree` — bracket principal + consolation (tailles 8/16/32)

## Génération matchs poule (`backend/crates/domain/src/generation.rs`)

`round_robin_pairs(teams)` = pur, toutes les paires non ordonnées (round-robin
simple, `n(n-1)/2`), ordre déterministe.

`App::generate_pool_matches(tid, pool_id)` : charge la poule (via `tournament_view`),
**idempotent** (saute les paires déjà planifiées, dédupe par paire non ordonnée),
émet un `Schedule` par paire (format = `pool_format`). Endpoint :
`POST /tournaments/{id}/pools/{pool_id}/matches` → `{created:[…]}`.
Front : bouton « Générer matchs (round-robin) » par poule.

## Bracket / phase finale (`backend/crates/domain/src/bracket.rs`)

Inspiré de l'original fouduvolant. Modèle event-sourced **lean** : seul le tirage
est persisté (mini-agrégat `Bracket`, command `Draw{main_seeds, consolation_seeds}`,
une fois). Tout l'arbre (main + consolante, tous les tours, byes, avancement) est
**reconstruit purement** par `build_bracket(main, cons, results)`, clé = paire
d'équipes non ordonnée. Pas de tree stocké → évite les pitfalls (id schemes,
seeding paths) de l'original.

- `bracket_size` : taille = puissance de 2 **≤ nb** (floor, `compute_final_bracket_size`
  de l'original). L'excès joue un **tour préliminaire** (round 0 = barrages
  principal / pré-tours consolante).
- **Play-in** : `extra = n − S`, `direct = S − extra` ; les `2*extra` plus faibles
  s'affrontent best-vs-worst (`seeds[direct+i]` vs `seeds[n-1-i]`), gagnants → slots
  restants. Pas de byes (slots toujours pleins) → un slot inconnu = gagnant
  préliminaire en attente, n'auto-avance pas.
- `seed_slots` : seeding standard, **identique** au `standard_seeding_order` de
  l'original (ex. taille 8 → [1,8,4,5,2,7,3,6]). Têtes de série séparées.
- **Consolante = équipes NON qualifiées** (bracket indépendant), pas les perdants
  du 1er tour — fidèle à l'original.
- `App::generate_bracket(tid, per_pool)` : top `per_pool`/poule → main (seeds
  rank-major, poules entrelacées) ; le reste → consolante. Draw + advance.
- `App::advance_bracket(tid)` : pull idempotent — planifie tout nœud jouable
  (2 équipes connues, pas encore programmé) en Match (pool None, format =
  `bracket_format`). Re-jouable après chaque résultat.
- `bracket_view(tid)` : arbre + noms. Endpoints : `POST …/bracket {per_pool}`,
  `POST …/bracket/advance`, `GET …/bracket`. Front : tirage + avancer + affichage
  principal/consolante.
- Play-in (barrages/pré-tours) : ✅ implémenté (round 0). Différé : 3e place (si taille ≥ 8).

## Scheduling (planner pur — `backend/crates/domain/src/scheduling.rs`)

Port propre du `court_dispatcher` legacy. **Fonction pure**, pas de wiring ES :
prend un snapshot `&[MatchView]` + terrains + `pool_court_map`, renvoie un
`CourtPlan` par terrain (`current`, `next`, `previews`).

- `assign_pools_to_courts` : mapping défaut greedy (1 poule → 1 terrain ; poules
  les plus chargées d'abord ; overflow partage les terrains les plus légers).
- `plan` : dispatch live. Cascade de sélection (legacy) :
  1. match épinglé manuellement (`manual_court`, le ▶) sur ce terrain
  2. poule préférée, équipes reposées (anti-btb)
  3. poule préférée, btb relâché (`needs_rest = true`)
  4. *(map vide seulement)* n'importe quel match reposé
  5. *(map vide seulement)* n'importe quel match
- **Anti-btb** : `recent` = équipes du match en cours (ou dernier fini) par
  terrain ; soft (relâché si inévitable, ex. poules de 4 → flag `needs_rest`).
  `busy` = équipes qui jouent maintenant = hard (jamais sélectionnables).
- **Weave** : à poules mélangées sur un terrain, priorité à la poule la moins
  complète (`done/total`), tiebreak `seq`. Étale les petites poules.
- **Idle-to-rest** : avec map explicite, un terrain sans poule assignée, sans
  épingle, sans historique → aucune suggestion.
- **Déterminisme** : tout ordre dérive de `MatchView.seq`. Aucun `HashMap` itéré
  pour une décision ordonnée.
- **Allocation 2 phases** (invariant) : phase 1 alloue le `next` de TOUS les
  terrains, phase 2 remplit les previews. Sinon le look-ahead d'un terrain
  consomme (`taken`) le `next` réel d'un autre → terrains affamés (bug corrigé).

Dispatch **hybride** : auto par défaut (map greedy, idle-to-rest off) ; manuel
via `manual_court` (overflow + réassignation).

### Projection `MatchView` (`backend/crates/domain/src/projections.rs`)
`MatchProjection` = read model qui folde les events `Match` (ordre global de
commit) en `MatchView` pour le planner. Synthétise depuis l'ordre global :
- `seq` (ordre de création, au 1er `Scheduled`)
- `done_order` (ordre de complétion, au `Completed`)

`manual_court` (le ▶) n'est pas un event `Match` → laissé `None`, posé via
`set_manual_court` quand câblé au côté commande scheduling.

### Process manager — `App::dispatch_courts(tournament_id)` (fait, pull)
Rejoue le store → projection → `plan` → `Start{court}` sur chaque terrain libre.
- `tournament_courts` : folde les events Tournament (dernier `CourtsConfigured`).
- `match_projection` : rejoue TOUS les events Match `ORDER BY global_seq`
  (ordre global de commit), puis filtre par `tournament`.
- N'auto-démarre PAS les suggestions `needs_rest` (btb forcé) → laissées au manuel
  (hybride). Retourne les matchs démarrés.
- **Pull** (appelé explicitement) : pas de réentrance, pas de chicken-egg cqrs-es.

### Reste au câblage ES (non fait)
- `global_seq` : colonne IDENTITY ajoutée à `events` (postgres-es ne l'écrit pas)
  pour l'ordre global. Voir `db/init.sql`.
- Évolution schéma events : un vieux payload incompatible casse le replay global
  (ex. `Scheduled` sans `tournament_id`) → upcasting via `event_version` à faire.
- Origine de `manual_court` : commande/aggregat scheduling pour le ▶.
- `MatchProjection` persisté comme `cqrs_es::Query` (au lieu de replay à chaque
  dispatch) si le volume l'exige — OK à l'échelle tournoi pour l'instant.

## Notes API cqrs-es 0.5
- `Aggregate` : `const TYPE`, pas de `aggregate_type()`.
- `handle(&mut self, cmd, services, sink: &EventSink<Self>) -> Result<(), Error>`
  (async natif, pas d'`async_trait`). On écrit via `sink.write(event, self)` qui
  **applique l'event immédiatement** puis l'enregistre.
- Erreurs : doivent impl `std::error::Error` (via `thiserror`).
