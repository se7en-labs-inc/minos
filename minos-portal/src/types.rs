use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Document types stored in sled
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Instance {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub expiration_ms: u64,
    pub payout_upperbound_ms: u64,
    pub vendor_expiration_ms: u64,
    pub treasury_hash: String,
    pub vendor_hash: String,
    pub treasury_ref: Option<TxOutRef>,
    pub vendor_ref: Option<TxOutRef>,
    pub publish_tx: String,
    pub network: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub instance_id: String,
    pub label: String,
    pub description: Option<String>,
    pub vendor_label: String,
    pub milestones: Vec<Milestone>,
    pub fund_tx: String,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub identifier: String,
    pub label: String,
    pub description: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub maturation_ms: u64,
    #[serde(default = "default_true")]
    pub requires_evidence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VendorUtxo {
    pub tx_hash: String,
    pub output_index: u64,
    pub project_id: String,
    pub address: String,
    pub datum_hex: String,
    pub lovelace: u64,
    pub assets: Vec<AssetAmount>,
    pub slot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetAmount {
    /// Empty string for lovelace; "<policyId><assetName>" for native assets
    pub asset_id: String,
    pub quantity: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxOutRef {
    pub transaction: String,
    pub index: u64,
}

// ---------------------------------------------------------------------------
// Event log types stored in DuckDB
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub tx_hash: String,
    pub block_slot: u64,
    pub block_height: u64,
    pub event_type: EventType,
    pub instance_id: String,
    pub project_id: Option<String>,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Publish,
    Fund,
    Complete,
    Withdraw,
    Pause,
    Resume,
    Modify,
    Cancel,
    Sweep,
    Initialize,
    Reorganize,
    Disburse,
}

impl EventType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "publish" => Some(Self::Publish),
            "fund" => Some(Self::Fund),
            "complete" => Some(Self::Complete),
            "withdraw" => Some(Self::Withdraw),
            "pause" => Some(Self::Pause),
            "resume" => Some(Self::Resume),
            "modify" => Some(Self::Modify),
            "cancel" => Some(Self::Cancel),
            "sweep" => Some(Self::Sweep),
            "initialize" => Some(Self::Initialize),
            "reorganize" => Some(Self::Reorganize),
            "disburse" => Some(Self::Disburse),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Fund => "fund",
            Self::Complete => "complete",
            Self::Withdraw => "withdraw",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Modify => "modify",
            Self::Cancel => "cancel",
            Self::Sweep => "sweep",
            Self::Initialize => "initialize",
            Self::Reorganize => "reorganize",
            Self::Disburse => "disburse",
        }
    }
}

// ---------------------------------------------------------------------------
// Derived milestone status (computed at query time)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MilestoneStatus {
    Active,
    Matured,
    Paused,
    Withdrawn,
    Cancelled,
}

// ---------------------------------------------------------------------------
// Completion evidence (extracted from Complete events)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionEvidence {
    pub anchor_url: String,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Completion {
    pub tx_hash: String,
    pub block_slot: u64,
    pub milestone_id: String,
    pub description: String,
    pub evidence: Vec<CompletionEvidence>,
}
