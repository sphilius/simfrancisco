# FORK_NOTES — repurposing the engine, and the first fork

Read [`ARCHITECTURE.md`](ARCHITECTURE.md) first; §4 there defines the seed-data
interface these notes build on.

Two candidate repurposings were compared:

- **(A) Synthetic READER/PLAYER audience** — poll simulated readers/players for
  reactions to a narrative beat or a game-design decision (extends prior work with
  reviewer datasets and value-shift tagging).
- **(B) Synthetic ArtsCenter/Carrboro community** — forecast workshop attendance and
  program demand (extends the ArtsCamp break-even tool).

## A vs B: adaptation difficulty

The engine's data-only seam is: *a census-shaped microdata CSV + a community TOML +
a rubric of questions*. Everything that is "people of a place, with demographics and
weights" flows through that seam untouched. Everything that isn't demographics
requires code.

| Dimension | (A) Reader/player audience | (B) ArtsCenter/Carrboro community |
|---|---|---|
| **Seed format** (`data/<slug>_pums.csv`) | Poor fit. Readers aren't census rows; you'd fabricate PUMS columns (PUMA, POVPIP, OCCP…) that don't describe a reader, or replace the ingest | **Native fit.** Carrboro literally has ACS PUMS rows (NC PUMA covering Orange County). Even a hand-built panel maps cleanly: age/education/income/kids-in-household are the real drivers of program demand |
| **Persona semantics** (`ValueVector`, `make_value_vector`, prose template — all Rust) | Wrong axes. Reader reactions need genre taste, tolerance for ambiguity/difficulty, spoiler sensitivity, value-shift tags — new schema, new prompt template, edits in `agent.rs`/`persona.rs` | Axes reused as-is. Political axes are harmless context; the **lifestyle layer** (hobbies incl. arts, routine, discretionary spending) is directly load-bearing for "would you pay for a pottery class" |
| **Prompt layer** (`vote/belief/options_prompt`) | `Options` framing is usable ("distribution over reactions"), but the electorate framing and `vote_facts` slot need rewording in code, not just data | Data-only: `vote_facts` paragraph re-frames YES as "your household actually registers and pays"; `Options` framing handles program-mix questions (which offering would you pick) unchanged |
| **What weights mean** | No natural `PWGTP`. What does one synthetic reader "stand for"? You'd have to invent an audience-composition model before any aggregate is meaningful | `PWGTP` is the point: `p_attend × Σ PWGTP` = **expected registrants**, which feeds the ArtsCamp break-even math directly. Demographic breakdowns (age band, income quintile) = program-targeting for free |
| **Forecast targets** (`rubric_<slug>.yaml`) | Ground truth is fuzzy (review sentiment? playtest surveys?) — hard to freeze, easy to leak | Ground truth exists and is yours: past seasons' actual enrollment/attendance per program. Freeze those as `target_share`, and `validate` becomes an honest backtest of the twin against real ArtsCenter history |
| **Analogy strength** | Polling mechanics carry over, but "city twin" priors (turnout, religion, homeownership) are dead weight | Same species as the original: geographic community + participation propensity. `turnout_propensity` is a worked template for an attendance-propensity layer |
| **Code changes for a first fork** | Persona schema redesign + prompt rework before the first honest poll | **Zero required** (this fork adds two tiny opt-in conveniences, see below) |

**Engine-fit verdict: (B) is the cleaner adaptation; (A) needs a persona-schema
scaffold first.** (B) exercises the engine exactly as designed — new TOML + new CSV +
new rubric, no code. (A) requires generalizing the persona layer (trait axes + prose
made data-driven, the way `CityProfile` already made the city data-driven).

**Decision: fork (A) is the chosen direction** (owner's call — the narrative/game-
design use is the priority). Both forks are now in-tree: (B)'s Carrboro re-seed was
built first as the zero-code proof of the seam and stays as a reference, and the
persona-schema scaffold that (A) requires is implemented below, validated end-to-end
the same way.

## The chosen fork (A): reader/player audience scaffold

**The persona schema** (`crates/sim-core/src/audience.rs` — the piece that didn't
exist): an `AudienceMember` is `{id, name, age, segment, weight, platforms[],
genres[], axes, values[], spoiler_tolerance, persona}` where

- `weight` is the **audience-composition weight** (the PWGTP analog): what share of
  the target audience this persona stands for. The example panel's weights sum to
  100, so every poll result reads directly as *% of the audience*.
- `axes` is a `TasteAxes` vector — the audience analog of the engine's `ValueVector`:
  `narrative_risk` (comfort-reads … wants-to-be-wrecked), `challenge`, `agency`
  (authored … player-driven), `tone` (cozy … grim), each in [-1, 1], rendered to
  natural language in the profile prose.
- `values` are free-form **value tags** (`found-family`, `consequences that stick`,
  `choices matter`, …) — the hook for the value-shift tagging work: they are what a
  beat honors or violates, and the natural mutable layer for future counterfactuals
  ("add foreshadowing, re-poll").
- `persona` is free prose appended verbatim — distill it straight from reviewer
  datasets.

**Wiring** (all opt-in, city paths untouched):

- `data/audiences/<slug>.json` holds the panel; a profile opts in via
  `audience_path` in its `data/cities/<slug>.toml` (`data/cities/readers.toml`).
- Audience populations are built by `audience::build_population` — every member
  exactly once with its weight (a panel is authored, not sampled).
- Polling clusters **one persona per archetype** (`predict.rs`): curated members are
  deliberately distinct, so demographic keys must never merge them. 12 members still
  fit one batched LLM call.
- `vote_prompt_override` (in `[politics]`) replaces the electorate framing with a
  reader/player framing: "estimate the probability this person answers YES … as they
  would actually react while reading or playing".
- The example first poll (`rubric_readers.yaml`) is a narrative-beat reception
  question — a fair-foreshadowed mentor-betrayal twist — with YES = "praises the
  twist in their review". Design-decision polls (difficulty modes, branching vs
  authored endings) ride the same path; multi-way questions can use the engine's
  existing `Options` framing via the API.

**Growing it from the reviewer datasets:** each reviewer cluster becomes a member
(segment = cluster label, `values` = its value-shift tags, `persona` = a distilled
exemplar review voice, `weight` = cluster share). Honest rubric targets = frozen
observed shares from shipped titles (e.g. positive-mention rate of a comparable twist
among reviews), never targets set after seeing the model's answer.

### The panel is genre-market-calibrated (K-lytics-style)

`data/audiences/readers.json` is no longer an ad-hoc example — its 16 members are the
major Kindle/KU fiction **genre-market reviewer clusters**, and each `weight` is that
segment's approximate share of the modeled reading market (weights sum to 100, so a
poll's `p_hat` reads directly as *% of the market* and per-cluster contribution is
just `weight × p_yes`). The weights are calibrated to **public** genre-market signals,
not scraped from proprietary K-lytics reports or live BSR/KU dashboards (no access to
either) — the file's `sources` array records what each figure rests on. Signals used:

- Romance dominates (~40%+ of Kindle sales, ~81% avg KU penetration); romance+fantasy
  are >60% of KU top earners → the romance bloc (contemporary, romantasy, dark, PNR,
  romantic suspense) is ~43% of panel weight.
- Romantasy/fantasy-romance now outsells traditional fantasy ~3:1 and leads demand →
  it's the single largest cluster (14).
- Fantasy ~13%, Sci-Fi ~8%, Mystery/Thriller ~20% of Kindle sales; LitRPG/progression
  is a KU-native, Royal-Road-fed segment; cozy mystery is older-female and KU-heavy;
  literary fiction is smaller, older, purchase/print-skewed and review-influential.
- Per-genre reader skews (gender/age/spoiler-tolerance) set each member's `axes` and
  demographics.

**To make it yours:** overwrite each `weight` with the market share from your K-lytics
subscription report, then replace/split segments and rewrite `persona`/`values` from
your own reviewer-dataset clusters. Nothing else changes; re-run `validate`. A parallel
*player* panel (Steam/itch genre analytics) would be a second `data/audiences/*.json`
with the same schema.

## Fork (B) reference re-seed (kept in-tree)

Built first to prove the data-only seam (no engine code touched):

- **`data/cities/carrboro.toml`** — community profile: Carrboro/Chapel Hill identity,
  progressive college-town value baselines, Orange-County-ish religion weights, and
  `vote_facts` re-framed for *participation* questions (YES = the household actually
  registers and pays; be honest about cost/time/childcare; most residents don't sign
  up for any given program).
- **`data/carrboro_pums.csv`** — a tiny example population: 48 adult rows in the
  standard 18-column PUMS schema, Σ PWGTP = 16,600 (≈ Carrboro's adult population).
  ⚠️ **Illustrative, hand-written rows** shaped like the real thing (grad students,
  UNC staff, artists, teachers, service workers, retirees) — *not* Census data.
  Replace with a real pull: NC PUMS `psam_p37.csv` filtered to the Orange County PUMA
  via `cargo run --bin ingest_pums -- --city carrboro --input data/pums/psam_p37.csv`
  (verify the current PUMA code for Carrboro/Chapel Hill — 2020 vintage ≈ 01301 —
  and update `pumas` in the TOML to match).
- **`rubric_carrboro.yaml`** — one end-to-end poll: *"does your household register
  and pay for ≥1 ArtsCenter class/workshop this fall?"* on `claude-sonnet-5`.
  ⚠️ `target_share: 0.10` is a **placeholder scaffold**, not ground truth — swap in
  real historical registration shares from the ArtsCamp data before treating scores
  as meaningful, and never tune prompts against a target you just set.

## Shared plumbing (small, default-off code changes)

In `crates/sim-core/src/model.rs` (used by both forks):

- `Model::Sonnet5` (`claude-sonnet-5`) so the fork polls on Sonnet 5; existing
  `claude-*` strings still resolve to the previous default as before.
- `DUMP_PROMPTS_DIR=<dir>` — on a cache miss, the client also writes the exact
  `{key, model, system, user, max_tokens}` to `<dir>/<cache-key>.json`. Combined with
  `MODEL_OFFLINE=1` this gives a **bring-your-own-LLM path**: dump prompts, answer
  them with any model/session you have access to, insert the answers into `cache.db`
  (`llm_cache(key, model, response, created)`), re-run — the engine then consumes
  them byte-reproducibly. This is how the end-to-end run below was produced in a
  sandbox with no API key; with `ANTHROPIC_API_KEY` set you skip it entirely.

## Run it

```bash
# deps: Rust toolchain (edition 2021, rust-version 1.80+); everything else is
# vendored via Cargo (rusqlite is bundled — no system sqlite needed). No tiles.db,
# no frontend, no network data needed for polling.

cargo test -p simfrancisco                    # engine unit + contract tests
cargo run --bin validate -- --city readers    # fork A: panel -> personas -> poll -> forecast
cargo run --bin validate -- --city carrboro   # fork B reference: PUMS -> poll -> forecast
```

API keys (only for live LLM calls; cached/offline re-runs need none):

| Key | Needed for |
|---|---|
| `ANTHROPIC_API_KEY` | `claude-sonnet-5` / `claude-sonnet-*` polls (this fork's path) |
| `MODEL_API_KEY` | Azure AI Foundry models (gpt-4o backtests, gpt-5.5, grok-4.3) — not needed for the Carrboro fork |
| `NEWS_API_KEY` | optional, only for the live news-refresh daemon |

Put keys in `.env` (git-ignored; see `.env.example`).

## Verification (run in this session)

Environment: sandbox, **no API keys**, so the LLM step used the bring-your-own-LLM
path with **Claude Sonnet 5** producing every archetype answer.

### Fork (A): genre-market reviewer panel

1. `cargo test -p simfrancisco` — **45 passed, 0 failed** (44 lib + 1 contract),
   including the audience-schema tests and a clustering test asserting curated panels
   never merge.
2. Prompt dump → **two** batched calls (16 members → 16 archetypes → 12 + 4);
   answered by Claude Sonnet 5; seeded into `cache.db`; re-run offline:

   ```
   ELECTION mentor_betrayal_twist_reception  pred=0.466 target=0.550 err=0.084 tol=0.150 score=0.72 PASS
   WEIGHTED HEADLINE = 0.7193  (gate ≥ 0.25)  PASS   llm: 0 calls, 2 cache hits
   ```

   Byte-identical across re-runs. **Because the panel weights are genre-market
   shares, the headline number is a market forecast: 46.6% of the modeled Kindle/KU
   fiction market would praise the mentor-betrayal beat in a review** — and the
   per-cluster split (contribution = weight × p_yes) is the actionable product:

   | Cluster (wt% of market) | p(praise) |
   |---|---|
   | grimdark 3 · epic fantasy 6 · dark romance 7 · psych thriller 8 · literary 4 | 0.88–0.92 (loves it) |
   | crime/fair-play 4 · sci-fi 5 · romantic suspense 4 | 0.60–0.80 |
   | **romantasy 14** (largest cluster) | **0.50 — the swing**, tolerates it only because the romance thread survives |
   | LitRPG 8 · urban fantasy 5 · YA 3 | 0.18–0.38 |
   | **contemporary romance 13 · cozy mystery 7 · cozy fantasy 4 · PNR 5** | 0.04–0.12 (hard no) |

   Net: **41% of the market loves the beat, 37% hates it** — a genuinely polarizing
   design decision, and the panel says *exactly which shelves* drive each side. The
   load-bearing insight: your two largest clusters (contemporary romance + romantasy =
   27% of the market) are lukewarm-to-hostile, so this beat risks read-through in the
   segments that matter most commercially — unless the romance thread is protected,
   which is precisely the lever that holds romantasy at the 0.50 swing line. The FAIL
   against the *placeholder* 0.55 target is the tool working; replace the target with a
   frozen reviewer-dataset share before reading the score as truth.

### Fork (B) reference: Carrboro

1. `cargo test -p simfrancisco` — all green (see above).
2. `MODEL_OFFLINE=1 DUMP_PROMPTS_DIR=… cargo run --bin validate -- --city carrboro`
   → dumped 4 batch prompts (48 agents → 40 archetypes → 4 batched calls).
3. Each prompt answered by Claude Sonnet 5 (exact system+user, strict-JSON
   `[{i, p_yes, why}]`), inserted into `cache.db` under the engine's own
   `sha256(model|system|user|max_tokens)` keys.
4. `MODEL_OFFLINE=1 cargo run --bin validate -- --city carrboro` — full pipeline on
   cache hits, identical across re-runs:

   ```
   ELECTION artscenter_fall_workshop_reg  pred=0.080 target=0.100 err=0.020 tol=0.080 score=0.87 PASS
   WEIGHTED HEADLINE = 0.8746  (gate ≥ 0.25)  PASS   llm: 0 calls, 4 cache hits
   ```

   Weighted forecast: **8.0%** of adult households (95% CI 6.8–9.2%, n_eff 47.3)
   → ×Σ PWGTP (16,600) ≈ **1,330 expected registering households**, the number the
   ArtsCamp break-even model consumes. Sonnet 5's per-archetype rationales were
   sensibly heterogeneous (retirees with leisure and working artists high; tight-
   budget renters and no-arts-hobby profiles low). The scorecard with pred/CI/n_eff
   lands in `runs/validate-<ts>/scorecard.json`; full demographic breakdowns
   (age/income/tenure) come back on the API's `POST /branches/{id}/poll`. Note the
   score itself is against the placeholder target — it proves the loop, not accuracy.

Reproduce live (with `ANTHROPIC_API_KEY`): `cargo run --bin validate -- --city readers`
(or `--city carrboro`) — same commands, no dump/seed steps.

## License / attribution

- This repo has **no top-level LICENSE file**, so default copyright applies to the
  original authors (Tejas & Tanmayi, per README/frontend credits). Keep this fork
  private, or get their OK (and a proper license) before publishing. Do not add a
  license on their behalf.
- Preserved as-is: `frontend/assets/SPRITES-LICENSE.txt` (CC BY-SA 3.0 sprite pack)
  and `crates/sim-maps/CREDITS.md` (OSM/DEM data credits). README attribution and
  the in-app builder credits are untouched.
- Data credits to carry into any fork: US Census ACS PUMS, Pew Religious Landscape
  Study, BLS ATUS + Consumer Expenditure Survey (embedded `data/survey/*.csv`).
- The example `data/carrboro_pums.csv` is fabricated for scaffolding and carries no
  third-party rights; a real NC PUMS pull is public-domain US Census data.
