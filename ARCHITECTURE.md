# ARCHITECTURE — how Sim Francisco works

A reverse-engineered map of the synthetic-population + polling + forecasting engine,
written for someone who wants to re-seed it with their own community or audience.
Companion doc: [`FORK_NOTES.md`](FORK_NOTES.md) (fork comparison + first-fork walkthrough).

The repo is a Rust workspace with two crates:

| Crate | Role |
|---|---|
| `crates/sim-core` | The engine: population seeding, polling, forecasting, HTTP API, life-sim |
| `crates/sim-maps` | OSM/DEM → pixel-tile map pipeline (visual layer only; **not needed for polling**) |

Inside `sim-core` there are really two engines over one shared persona layer
(`crates/sim-core/src/lib.rs`):

1. **Prediction engine** (`predict.rs` + `aggregate.rs` + `rubric.rs`) — persona +
   as-of-date + event → weighted opinion/vote/probability. Runs headless, no map, no
   life-sim. This is the part worth forking.
2. **Life simulation** (`sim.rs`, `pathfind.rs`, `geo.rs`, `state.rs`, `store.rs`) —
   sprites moving on a tile grid, SSE-streamed to the vanilla-JS frontend. Eye candy;
   fully optional (every entry point takes `tiles: Option<&TilesDb>` and `validate`
   passes `None`).

---

## 1. How the population is seeded

The pipeline is: **real Census microdata → sampled agents → deterministic personas →
value vectors → prompt-ready prose**. No LLM is used at seeding time; everything is
seeded RNG, so a population is byte-reproducible from `(data, seed, n)`.

### 1.1 Microdata ingest — `pums.rs`

- Source of truth is an **ACS PUMS person-microdata CSV** (one row = one real,
  anonymized survey respondent). The committed per-city subsets live at
  `data/<slug>_pums.csv` with exactly the 18 columns in `pums::KEEP_COLS`:
  `SERIALNO, SPORDER, PWGTP, AGEP, SEX, RAC1P, HISP, SCHL, PINCP, POVPIP, OCCP, COW,
  ESR, CIT, MAR, NATIVITY, PUMA, ADJINC`.
- `PWGTP` is the survey weight: "this respondent stands for ~N real people". It is
  carried onto every agent and used in **every** population estimate, which is why a
  few hundred agents can represent a whole city — and why `p_hat × Σ PWGTP` is a
  real-population headcount.
- The key idea (BRIEF §3.1): because PUMS rows are *joint* samples, the joint
  distribution over age × race × education × income × occupation × tenure comes for
  free — no synthesis from marginals. A `marginals_match_population` test
  (`persona.rs`) asserts the weighted sample reproduces the source marginals.
- `ingest_pums` (bin) filters a full-state PUMS file down to a city's PUMA codes and
  writes the small committed subset: `cargo run --bin ingest_pums -- --city <slug>
  --input data/pums/psam_pXX.csv`.

### 1.2 City profile — `city.rs` (the *other half* of the seed)

Everything city-specific that microdata can't provide lives in
`data/cities/<slug>.toml`, deserialized into `CityProfile`:

- identity: `slug`, `display`, `prompt_name`, `demonym`
- geography: `pumas` (which microdata rows belong), `neighborhoods` (PUMA → label),
  `centroids` (map placement; unused without tiles), `work` clustering
- `religion_weights` — Pew metro shares, layered onto agents in `religion.rs`
  conditioned on demographics (Census has no religion field)
- `politics` — baseline value-vector means (`economic_base`, `social_base`, …),
  issue-salience baselines, and two **prompt paragraphs**: `vote_facts` and
  `belief_facts`, injected verbatim into the LLM system prompts. This is where a
  community's "personality" and ground rules are told to the model.

`CityProfile::sf()` is hardcoded; every other city loads from TOML. The module doc
states the contract explicitly: *"Adding a city = drop a `data/cities/<slug>.toml` +
its PUMS subset + tiles.db. No code."* (tiles only needed for the visual sim).

### 1.3 Agent + persona generation — `persona.rs`, `agent.rs`, `lifestyle.rs`

`build_population_with(records, n, seed, tiles, profile)`:

1. Sample `n` record indices with a ChaCha8 RNG seeded by `seed` (without replacement
   when `n ≤ records.len()`, else with).
2. Compute PWGTP-weighted income-quintile cutoffs over the sample (`POVPIP`
   income-to-poverty ratio is the economic-standing scalar).
3. For each agent, derive a per-agent seed = `sha256(sim_seed, agent_index)` and from
   it, deterministically:
   - **religion + religiosity** (`religion.rs`: city baseline weights tilted by
     race/age/education),
   - **homeownership** (logistic in income/age/marriage),
   - a **`ValueVector`** (`agent.rs`): 4 opinion axes in [-1,1] (`economic`, `social`,
     `trust`, `change`) + 6 issue saliences in [0,1] (`s_housing`, `s_crime`,
     `s_homeless`, `s_cost`, `s_environment`, `s_immigration`). Computed as city
     baseline + demographic deltas + small seeded noise (`make_value_vector`).
   - a **name** (sex/race-conditioned lists), an **occupation label** (OCCP code
     ranges), a **neighborhood** (PUMA lookup),
   - a **lifestyle** (`lifestyle.rs`): hobbies, daily routine, spending tilt drawn
     from embedded BLS ATUS / CEX / hobby survey tables (`data/survey/*.csv`) —
     notably including arts-adjacent hobbies,
   - **persona prose** (`build_persona_prose`): one paragraph stitching all of the
     above together, ending with `values.describe()` (a natural-language rendering of
     the value vector) and the lifestyle sentence. **This paragraph is the only thing
     the LLM ever sees about an agent.**

Determinism is treated as a hard invariant (tests: `deterministic_population`); the
lifestyle draw is even ordered last so adding it didn't disturb earlier draws.

---

## 2. How polling works

`predict.rs` — the scored core. A `Poll` is
`{question, description, framing, as_of_date, model?, population?, event?, options?}`.

### 2.1 Framings (three prompt families, `city.rs`)

| Framing | Prompt | Output per agent | Use |
|---|---|---|---|
| `Vote` | `vote_prompt()` — "reason as this resident; probability THIS resident votes YES" + `vote_facts` | `p_yes` | Elections, measures, any yes/no intent |
| `Belief` | `belief_prompt()` — "calibrated forecaster; probability the event happens" + `belief_facts` | `p_yes` | Prediction markets |
| `Options` | `options_prompt()` — "distribution over N labelled options, ground in the persona's lifestyle/tastes, not stereotypes" | `dist[]` | Preferences, multi-candidate, **non-political questions** |

`as_of_date` is first-class: prompts instruct "use ONLY knowledge available on this
date". Recent-dated polls (≥ 2025-06-01) additionally get a news block from
`data/news/<slug>.json` (`news.rs`); backtests never see it.

### 2.2 Cost control: archetype clustering + post-stratification

The LLM is **not** called once per agent. `cluster_agents()` groups agents by a
demographic archetype key (`age_band | race | educ | income_q | tenure | citizen`),
coarsening the key until ≤ `MAX_CLUSTERS` (default 160, env-overridable). One
representative persona per archetype is sent, ~12 archetypes per batched call
(`batch_size = 12`), i.e. a whole-city poll is ~14 LLM calls. The model returns
`[{i, p_yes, why}]` (or `{i, dist, why}` for Options); every member of an archetype
inherits its probability; then results are **post-stratified with PUMS weights** — a
standard synthetic-survey estimator. Cluster order is sorted for determinism so
prompts (and therefore cache keys) are identical across runs.

### 2.3 Model client + cache — `model.rs`

`ModelClient` speaks three provider shapes over raw `reqwest`: Azure `/responses`
(gpt-4o, gpt-5.5), Azure `/chat/completions` (grok-4.3), and the Anthropic Messages
API (`claude-sonnet-*`, keyed by `ANTHROPIC_API_KEY`). Semaphore-bounded concurrency,
429/5xx backoff, and a **sqlite response cache** (`cache.db`) keyed by
`sha256(model | system | user | max_tokens)`. The cache is what makes "clean mode"
byte-reproducible and free on re-run; `MODEL_OFFLINE=1` disables the network so only
cache hits resolve. (This fork adds `DUMP_PROMPTS_DIR` — see FORK_NOTES — which
writes each cache-miss prompt to disk so answers can be generated out-of-band and
seeded into the cache.)

### 2.4 Aggregation math — `aggregate.rs`

Pure, unit-tested functions:

- `p_hat(k) = Σ_i w_i · a_i(k) / Σ_i w_i` (Horvitz-Thompson ratio; soft indicators
  allowed, so an archetype's conditional probability slots in directly)
- Kish `effective_n` / `design_effect`, weighted bootstrap CI (deterministic given
  seed), per-demographic `breakdown` (age, race, educ, income quintile, PUMA, tenure)
- For elections, the population is restricted to citizen voting-age agents and
  weights are multiplied by `turnout_propensity(agent)` — a documented logistic in
  age/education/income/tenure/marriage (`predict.rs`). This is the template for any
  "propensity to show up" layer (e.g. workshop-attendance propensity).

### 2.5 Events / counterfactuals

An `Event {text, as_of_date}` is prepended to the poll prompt ("recent event everyone
is aware of"). `run_counterfactual` polls baseline and with-event, returning
`(baseline, after, delta)` — scored on direction, not fabricated magnitude.

---

## 3. How forecasts are generated & validated

### 3.1 The rubric — `rubric.yaml` + `rubric.rs`

A forecast run is defined declaratively: `meta` (validation_n, seed), `weights`
(per category), `thresholds`, and entries in three categories:

- `elections_measures`: Vote polls scored by absolute error vs a frozen ground-truth
  `target_share` (score 1.0 at zero error, 0.5 at tolerance, 0 at 2×).
- `resolved_markets`: Belief polls scored by **Brier** vs the resolved outcome,
  bucketed `sf_opinion_informative` (scored) vs `general_knowledge` (reported,
  weight 0).
- `counterfactuals`: direction correctness + optional magnitude penalty.

Per-city rubrics exist (`rubric_<slug>.yaml`), so the scoreboard swaps with the seed.

### 3.2 The `validate` binary

`cargo run --bin validate -- --city <slug>` = the whole pipeline, headless:
load rubric → load `CityProfile` → load PUMS subset → `build_population_with(…,
tiles=None)` → run every entry's poll in clean mode at its as-of-date/model →
score → print scorecard → write `runs/validate-<ts>/scorecard.json` → exit 0 iff
weighted headline ≥ gate. Flags: `--smoke` (N≤400), `--n`, `--seed`, `--rubric`,
`--out`, `--quiet`.

### 3.3 The HTTP API — `api.rs` + `server` bin

For interactive use (`POST /simulations` → `POST /simulations/{id}/branches` →
`POST /branches/{id}/poll`), plus `/demographics` (sampled marginals vs targets),
`/predict-market`, `/branches/{id}/stream` (SSE for the frontend), `/cities`,
`/health`. The server loads SF plus a **hardcoded list** of extra cities
(`build_state`: `["neu_york", "synth_la", "cybercago", "simami"]`) — one line to
extend for a new slug. The life-sim (`sim.rs`) only matters here, feeding sprite
positions and chatter to `frontend/`.

### 3.4 Leakage discipline (worth keeping in any fork)

Backtests pin models with knowledge cutoffs that predate the target event; prompt
context is restricted to pre-cutoff priors; targets are frozen public ground truth;
zero-weight buckets are reported but never inflate the headline. The same discipline
maps to any forecasting fork: freeze your ground truth (e.g. last season's actual
enrollment) before tuning prompts.

---

## 4. The seed-data interface (exactly what to swap)

To re-seed the engine with your own population, you touch **data only**:

| # | File | What it encodes |
|---|---|---|
| 1 | `data/<slug>_pums.csv` | The population: one row per person/persona, 18 PUMS columns, `PWGTP` = how many real people this row stands for, `PUMA` = geography bucket |
| 2 | `data/cities/<slug>.toml` | Community identity, value-vector baselines, issue saliences, and the two system-prompt paragraphs (`vote_facts`, `belief_facts`) |
| 3 | `rubric_<slug>.yaml` | The questions you poll + frozen targets + scoring gates |
| 4 | `data/news/<slug>.json` | *(optional)* current-events context for live-dated polls |
| 5 | `data/survey/*.csv` | *(optional, shared)* hobby/routine/spending tables behind the lifestyle layer |

Then: `cargo run --bin validate -- --city <slug>`. No tiles.db, no code — unless you
also want the city in the HTTP server (one line in `api.rs::build_state`) or on the
pixel map (full `sim-maps` pipeline).

The **hard boundary** of the data-only interface: persona *semantics* live in code.
`ValueVector`'s axes, `make_value_vector`'s demographic deltas, `turnout_propensity`,
and the persona-prose template are Rust (`agent.rs`, `persona.rs`, `predict.rs`). A
fork whose personas are still "people in a place with demographics" never hits this
boundary; a fork that needs different *kinds* of traits (e.g. genre taste axes) does.
That boundary is what decides the fork comparison in `FORK_NOTES.md`.
