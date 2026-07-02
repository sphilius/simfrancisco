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

**Recommendation: fork (B) first.** It exercises the engine exactly as designed —
new TOML + new CSV + new rubric, no code — so you validate the re-seed path itself
before bending any semantics. (A) is a second-generation fork: once (B) proves the
loop, (A) needs a persona-schema generalization (trait axes + prose template made
data-driven, the way `CityProfile` already made the city data-driven). Doing (A)
first means doing that refactor blind.

## The first concrete change (this branch)

Data (the actual re-seed — no engine code touched):

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

Code (two small, default-off conveniences in `crates/sim-core/src/model.rs`):

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

cargo test -p simfrancisco                 # engine unit + contract tests
cargo run --bin validate -- --city carrboro   # seed -> personas -> poll -> forecast
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
path with **Claude Sonnet 5** producing every archetype answer:

1. `cargo test -p simfrancisco` — **41 passed, 0 failed** (40 lib + 1 contract).
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

Reproduce live (with `ANTHROPIC_API_KEY`): `cargo run --bin validate -- --city carrboro`
— same commands, no dump/seed steps.

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
