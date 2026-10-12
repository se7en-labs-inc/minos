/// Parse raw CIP-100 metadata (as returned by Koios tx_metadata) into typed events.
///
/// Koios returns metadata as a JSON object with string keys. The 1694 key holds
/// the decoded metadata map. We extract the body.event field and route accordingly.
use anyhow::Result;
use serde_json::Value;

use crate::types::{Event, EventType, Milestone, Project};

pub struct ParsedEvent {
    pub event: Event,
    /// If this is a fund event, the parsed project (milestones embedded)
    pub project: Option<Project>,
}

/// Try to parse a CIP-100 event from raw Koios metadata.
/// Returns None if the metadata is missing, malformed, or not a known event type.
pub fn parse_metadata(
    tx_hash: &str,
    block_slot: u64,
    block_height: u64,
    raw: &Value,
) -> Option<ParsedEvent> {
    // Koios wraps metadata as { "1694": { ... } } or as a decoded object directly
    let meta = raw.get("1694").unwrap_or(raw);

    let instance_id = meta.get("instance")?.as_str()?.to_string();
    let body = meta.get("body")?;
    let event_str = body.get("event")?.as_str()?;
    let event_type = EventType::from_str(event_str)?;

    let project_id = extract_project_id(body, event_type);

    let event = Event {
        tx_hash: tx_hash.to_string(),
        block_slot,
        block_height,
        event_type,
        instance_id: instance_id.clone(),
        project_id: project_id.clone(),
        payload: body.clone(),
    };

    let project = if event_type == EventType::Fund {
        parse_fund_project(&instance_id, tx_hash, body)
    } else {
        None
    };

    Some(ParsedEvent { event, project })
}

fn extract_project_id(body: &Value, event_type: EventType) -> Option<String> {
    match event_type {
        EventType::Fund | EventType::Modify => body.get("identifier")?.as_str().map(str::to_string),
        EventType::Complete
        | EventType::Withdraw
        | EventType::Pause
        | EventType::Resume
        | EventType::Cancel
        | EventType::Sweep => {
            // These events carry a project identifier in various places;
            // Sweep has projectIdentifier, others use identifier or are
            // associated by the vendor UTxO address (resolved by indexer).
            body.get("identifier")
                .or_else(|| body.get("projectIdentifier"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        }
        _ => None,
    }
}

fn parse_fund_project(instance_id: &str, fund_tx: &str, body: &Value) -> Option<Project> {
    let id = body.get("identifier")?.as_str()?.to_string();
    let label = rejoined_string(body.get("label")?);
    let description = body.get("description").map(rejoined_string);
    let vendor_label = body
        .get("vendor")
        .and_then(|v| v.get("label"))
        .map(rejoined_string)
        .unwrap_or_default();

    let milestones = parse_milestones(body)?;

    Some(Project {
        id,
        instance_id: instance_id.to_string(),
        label,
        description,
        vendor_label,
        milestones,
        fund_tx: fund_tx.to_string(),
    })
}

fn parse_milestones(body: &Value) -> Option<Vec<Milestone>> {
    // Milestones can be a map (spec v2: {"id": {...}}) or an array (older format)
    let ms_val = body.get("milestones")?;

    if let Some(map) = ms_val.as_object() {
        let mut milestones: Vec<Milestone> = map
            .iter()
            .filter_map(|(id, val)| {
                Some(Milestone {
                    identifier: id.clone(),
                    label: rejoined_string(val.get("label")?),
                    description: val.get("description").map(rejoined_string),
                    acceptance_criteria: val.get("acceptanceCriteria").map(rejoined_string),
                    maturation_ms: 0, // filled from on-chain datum
                    requires_evidence: val
                        .get("requiresEvidence")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                })
            })
            .collect();
        // Preserve insertion order as display order
        milestones.sort_by(|a, b| a.identifier.cmp(&b.identifier));
        return Some(milestones);
    }

    if let Some(arr) = ms_val.as_array() {
        let milestones = arr
            .iter()
            .filter_map(|val| {
                Some(Milestone {
                    identifier: val.get("identifier")?.as_str()?.to_string(),
                    label: rejoined_string(val.get("label")?),
                    description: val.get("description").map(rejoined_string),
                    acceptance_criteria: val.get("acceptanceCriteria").map(rejoined_string),
                    maturation_ms: 0,
                    requires_evidence: val
                        .get("requiresEvidence")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                })
            })
            .collect();
        return Some(milestones);
    }

    None
}

/// Reconstruct a string that may have been split into ≤64-byte chunks as a CBOR array.
pub fn rejoined_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(chunks) => chunks
            .iter()
            .filter_map(|c| c.as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => v.to_string(),
    }
}
