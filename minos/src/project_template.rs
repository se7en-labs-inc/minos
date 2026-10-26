//! Clone public project metadata and amounts with a controlled preview schedule.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Option<String>,
    #[serde(default = "default_evidence")]
    pub requires_evidence: bool,
    pub maturation_ms: u64,
    #[serde(default)]
    pub lovelace: u64,
    #[serde(default)]
    pub token_amount: u64,
}
fn default_evidence() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectTemplate {
    pub project: String,
    pub label: String,
    pub description: String,
    pub vendor_label: String,
    pub token_policy: Option<String>,
    pub token_name: Option<String>,
    pub milestones: Vec<Milestone>,
}

pub fn clone_project(
    snapshot: &Value,
    interval_minutes: u64,
    now_ms: u64,
    token_policy: Option<String>,
    token_name: Option<String>,
) -> Result<ProjectTemplate> {
    clone_project_inner(
        snapshot,
        interval_minutes,
        now_ms,
        token_policy,
        token_name,
        None,
    )
}

pub fn clone_project_ada(
    snapshot: &Value,
    interval_minutes: u64,
    now_ms: u64,
    ada_per_milestone: u64,
) -> Result<ProjectTemplate> {
    ensure!(ada_per_milestone > 0, "ADA payout must be positive");
    let lovelace = ada_per_milestone
        .checked_mul(1_000_000)
        .context("ADA payout overflow")?;
    clone_project_inner(
        snapshot,
        interval_minutes,
        now_ms,
        None,
        None,
        Some(lovelace),
    )
}

fn clone_project_inner(
    snapshot: &Value,
    interval_minutes: u64,
    now_ms: u64,
    token_policy: Option<String>,
    token_name: Option<String>,
    fixed_lovelace: Option<u64>,
) -> Result<ProjectTemplate> {
    ensure!(interval_minutes > 0, "unlock interval must be positive");
    ensure!(
        token_policy.is_some() == token_name.is_some(),
        "provide both token policy and name"
    );
    if let Some(policy) = &token_policy {
        ensure!(hex::decode(policy)?.len() == 28, "invalid token policy");
    }
    if let Some(name) = &token_name {
        ensure!(hex::decode(name)?.len() <= 32, "invalid token name");
    }
    let p = &snapshot["instanceById"]["project"];
    let string = |v: &Value| -> Result<String> {
        Ok(v.as_str().context("missing project text")?.to_owned())
    };
    let mut rows: Vec<_> = p["milestones"]
        .as_array()
        .context("missing source milestones")?
        .iter()
        .collect();
    rows.sort_by_key(|m| m["contractMilestoneNumber"].as_u64().unwrap_or(u64::MAX));
    ensure!(!rows.is_empty(), "source has no milestones");
    let interval = interval_minutes
        .checked_mul(60_000)
        .context("interval overflow")?;
    let mut source_asset: Option<String> = None;
    let mut milestones = Vec::new();
    let mut ids = std::collections::BTreeSet::new();
    for (index, m) in rows.iter().enumerate() {
        let id = string(&m["identifier"])?;
        ensure!(ids.insert(id.clone()), "duplicate source milestone");
        let mut lovelace = 0u64;
        let mut token_amount = 0u64;
        for value in m["value"].as_array().context("missing milestone value")? {
            let asset = value["assetId"].as_str().context("missing asset ID")?;
            let quantity = value["quantity"]
                .as_str()
                .context("missing quantity")?
                .parse::<u64>()?;
            if asset.is_empty() || asset == "ada.lovelace" {
                lovelace = lovelace
                    .checked_add(quantity)
                    .context("ADA amount overflow")?;
            } else {
                ensure!(
                    source_asset.as_ref().is_none_or(|a| a == asset),
                    "only one source token supported"
                );
                source_asset = Some(asset.to_owned());
                token_amount = token_amount
                    .checked_add(quantity)
                    .context("token amount overflow")?;
            }
        }
        let maturation_ms = now_ms
            .checked_add(
                interval
                    .checked_mul(index as u64)
                    .context("schedule overflow")?,
            )
            .context("schedule overflow")?;
        milestones.push(Milestone {
            id,
            label: string(&m["label"])?,
            description: m["description"].as_str().map(str::to_owned),
            acceptance_criteria: m["acceptanceCriteria"].as_str().map(str::to_owned),
            requires_evidence: m["requiresEvidence"].as_bool().unwrap_or(true),
            maturation_ms,
            lovelace: fixed_lovelace.unwrap_or(lovelace),
            token_amount: if fixed_lovelace.is_some() {
                0
            } else {
                token_amount
            },
        });
    }
    ensure!(
        fixed_lovelace.is_some() || source_asset.is_none() || token_policy.is_some(),
        "source uses tokens; explicitly map a preview test token"
    );
    Ok(ProjectTemplate {
        project: string(&p["identifier"])?,
        label: string(&p["label"])?,
        description: string(&p["description"])?,
        vendor_label: string(&p["vendor"]["label"])?,
        token_policy,
        token_name,
        milestones,
    })
}
