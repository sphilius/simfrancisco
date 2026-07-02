//! Audience personas: the seed layer for non-geographic populations (fork A).
//!
//! Where the city path seeds agents from Census microdata (`pums.rs` + `persona.rs`),
//! this path seeds a curated READER/PLAYER panel from `data/audiences/<slug>.json`.
//! Each member carries an audience-composition `weight` (the PWGTP analog: what share
//! of the target audience this persona stands for), a compact [`TasteAxes`] vector
//! (the audience analog of `ValueVector`), and value tags for value-shift work.
//! Everything downstream — clustering, batched polling, weighted aggregation, the
//! rubric — is reused unchanged; a profile opts in via `audience_path` in its
//! `data/cities/<slug>.toml`.

use crate::agent::{Agent, ValueVector};
use crate::city::CityProfile;
use crate::geo::Cell;
use crate::persona::{agent_seed, Population};
use crate::pums::PumsRecord;
use crate::religion::Religion;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudienceFile {
    /// Audience slug (matches the profile that references this file).
    pub audience: String,
    pub members: Vec<AudienceMember>,
}

/// One audience persona. Prose the LLM sees is composed from these fields plus the
/// free-text `persona` (e.g. distilled straight from a reviewer dataset).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AudienceMember {
    pub id: String,
    pub name: String,
    pub age: u8,
    /// Short segment label, e.g. "BookTok romantasy devotee".
    pub segment: String,
    /// Audience-composition weight: what share of the target audience this persona
    /// represents. Any positive scale works; p_hat is weight-normalized.
    pub weight: f64,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub axes: TasteAxes,
    /// Value tags — what this person wants stories/games to honor (the value-shift
    /// layer: events and beats can move these in future counterfactual work).
    #[serde(default)]
    pub values: Vec<String>,
    /// 0 = spoiler-averse … 1 = spoiler-immune.
    #[serde(default = "default_spoiler")]
    pub spoiler_tolerance: f64,
    /// Free-prose backstory appended to the composed profile.
    #[serde(default)]
    pub persona: String,
}

fn default_spoiler() -> f64 {
    0.5
}

/// Compact taste vector for readers/players. Axes in [-1, 1].
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct TasteAxes {
    /// -1 comfort reads / safe arcs … +1 wants risky twists, happy to be hurt
    #[serde(default)]
    pub narrative_risk: f64,
    /// -1 casual … +1 hardcore challenge
    #[serde(default)]
    pub challenge: f64,
    /// -1 authored/linear … +1 player-driven/branching
    #[serde(default)]
    pub agency: f64,
    /// -1 light/cozy … +1 grim/dark
    #[serde(default)]
    pub tone: f64,
}

impl TasteAxes {
    /// Natural-language rendering for prompts (mirrors `ValueVector::describe`).
    pub fn describe(&self) -> String {
        format!(
            "{}; {}; {}; {}",
            axis_word(
                self.narrative_risk,
                "prefers comfort reads and safe arcs",
                "enjoys some narrative risk",
                "wants risky twists and is happy to be emotionally wrecked"
            ),
            axis_word(
                self.challenge,
                "casual about difficulty",
                "likes a moderate challenge",
                "seeks hardcore challenge"
            ),
            axis_word(
                self.agency,
                "prefers authored, linear stories",
                "flexible between authored and player-driven",
                "wants branching, player-driven control"
            ),
            axis_word(
                self.tone,
                "leans light, warm, and cozy in tone",
                "tone-flexible",
                "leans grim and dark in tone"
            ),
        )
    }
}

fn axis_word(v: f64, low: &'static str, mid: &'static str, high: &'static str) -> &'static str {
    if v < -0.33 {
        low
    } else if v > 0.33 {
        high
    } else {
        mid
    }
}

/// Load an audience file and return its members.
pub fn load(path: &str) -> Result<Vec<AudienceMember>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("open audience file {path}"))?;
    let f: AudienceFile = serde_json::from_str(&text).with_context(|| format!("parse audience file {path}"))?;
    if f.members.is_empty() {
        return Err(anyhow!("audience file {path} has no members"));
    }
    Ok(f.members)
}

/// The one profile paragraph the LLM sees for this member.
pub fn prose(m: &AudienceMember) -> String {
    let mut s = format!("{}, {}, {}.", m.name, m.age, m.segment);
    if !m.platforms.is_empty() {
        s.push_str(&format!(" Reads/plays via {}.", m.platforms.join(", ")));
    }
    if !m.genres.is_empty() {
        s.push_str(&format!(" Favorite genres: {}.", m.genres.join(", ")));
    }
    s.push_str(&format!(" Tastes: {}.", m.axes.describe()));
    let spoil = if m.spoiler_tolerance < 0.33 {
        "low (avoids spoilers)"
    } else if m.spoiler_tolerance > 0.67 {
        "high (unbothered by spoilers)"
    } else {
        "moderate"
    };
    s.push_str(&format!(" Spoiler tolerance: {spoil}."));
    if !m.values.is_empty() {
        s.push_str(&format!(" Values in stories: {}.", m.values.join(", ")));
    }
    if !m.persona.is_empty() {
        s.push(' ');
        s.push_str(m.persona.trim());
    }
    s
}

/// Neutral synthetic record: carries the member's weight and age so weighted
/// aggregation and age-band breakdowns work; every other field is inert.
fn synth_record(m: &AudienceMember) -> PumsRecord {
    PumsRecord {
        serialno: m.id.clone(),
        sporder: 1,
        pwgtp: m.weight.max(1e-6),
        age: m.age,
        sex: 1,
        rac1p: 1,
        hisp: 1,
        schl: 21,
        pincp: 0.0,
        povpip: 0.0,
        occp: 0,
        cow: 0,
        esr: 6,
        cit: 1,
        mar: 5,
        nativity: 1,
        puma: 0,
        adjinc: 1.0,
    }
}

/// Build a Population from a curated panel: every member exactly once, carrying its
/// weight. Deterministic; no sampling (a panel is authored, not drawn).
pub fn build_population(members: &[AudienceMember], seed: u64, profile: Arc<CityProfile>) -> Population {
    let zero = ValueVector {
        economic: 0.0,
        social: 0.0,
        trust: 0.0,
        change: 0.0,
        s_housing: 0.0,
        s_crime: 0.0,
        s_homeless: 0.0,
        s_cost: 0.0,
        s_environment: 0.0,
        s_immigration: 0.0,
    };
    let agents: Vec<Agent> = members
        .iter()
        .enumerate()
        .map(|(i, m)| Agent {
            id: i as u32,
            rec: synth_record(m),
            seed: agent_seed(seed, i as u32),
            name: m.name.clone(),
            religion: Religion::Unaffiliated,
            religiosity: 0.0,
            homeowner: false,
            values: zero,
            persona: prose(m),
            occupation: m.segment.clone(),
            neighborhood: profile.prompt_name.clone(),
            home: Cell::new(0, 0),
            work: None,
        })
        .collect();
    let n = agents.len();
    Population {
        agents,
        income_cutoffs: [0.0; 4],
        seed,
        n,
        profile,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, w: f64) -> AudienceMember {
        AudienceMember {
            id: id.into(),
            name: format!("R {id}"),
            age: 30,
            segment: "cozy-fantasy comfort reader".into(),
            weight: w,
            platforms: vec!["Kindle".into()],
            genres: vec!["cozy fantasy".into()],
            axes: TasteAxes { narrative_risk: -0.9, challenge: -0.6, agency: -0.3, tone: -0.9 },
            values: vec!["kindness-rewarded".into()],
            spoiler_tolerance: 0.2,
            persona: "Reads nightly to decompress.".into(),
        }
    }

    #[test]
    fn schema_parses_with_defaults() {
        let json = r#"{"audience":"t","members":[{"id":"a","name":"A","age":25,"segment":"s","weight":10}]}"#;
        let f: AudienceFile = serde_json::from_str(json).unwrap();
        let m = &f.members[0];
        assert_eq!(m.spoiler_tolerance, 0.5);
        assert!(m.genres.is_empty() && m.values.is_empty());
        assert_eq!(m.axes.narrative_risk, 0.0);
    }

    #[test]
    fn prose_reflects_the_schema() {
        let p = prose(&member("a", 10.0));
        assert!(p.contains("cozy-fantasy comfort reader"));
        assert!(p.contains("comfort reads"));
        assert!(p.contains("low (avoids spoilers)"));
        assert!(p.contains("kindness-rewarded"));
        assert!(p.contains("Reads nightly"));
    }

    #[test]
    fn panel_population_is_exact_and_weighted() {
        let members: Vec<AudienceMember> = (0..5).map(|i| member(&format!("m{i}"), 10.0 + i as f64)).collect();
        let profile = Arc::new(CityProfile::sf());
        let pop = build_population(&members, 42, profile.clone());
        assert_eq!(pop.agents.len(), 5);
        assert!((pop.total_weight() - (10.0 + 11.0 + 12.0 + 13.0 + 14.0)).abs() < 1e-9);
        // deterministic + order-preserving (panel order == agent order)
        let pop2 = build_population(&members, 42, profile);
        for (a, b) in pop.agents.iter().zip(pop2.agents.iter()) {
            assert_eq!(a.persona, b.persona);
            assert_eq!(a.rec.serialno, b.rec.serialno);
        }
        assert_eq!(pop.agents[3].rec.serialno, "m3");
    }
}
