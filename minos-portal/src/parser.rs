/// Parse raw CIP-100 metadata (as returned by Koios tx_metadata) into typed events.
///
/// Koios returns metadata as a JSON object with string keys. The 1694 key holds
/// the decoded metadata map. We extract the body.event field and route accordingly.
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

    let instance_id = meta.get("instance")?.as_str()?.to_ascii_lowercase();
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
    if id.trim().is_empty() {
        return None;
    }
    let label = checked_string(body.get("label")?)?;
    let description = optional_text(body.get("description"))?;
    let vendor_label = body
        .get("vendor")
        .and_then(|v| v.get("label"))
        .and_then(checked_string)?;

    let milestones = parse_milestones(body)?;

    Some(Project {
        id,
        instance_id: instance_id.to_string(),
        label,
        description,
        vendor_label,
        milestones,
        fund_tx: fund_tx.to_string(),
        ordered_milestones: body.get("milestones").is_some_and(Value::is_array),
    })
}

fn parse_milestones(body: &Value) -> Option<Vec<Milestone>> {
    // The pinned format is an ordered list. Legacy maps retain their IDs but
    // cannot establish payout order merely from JSON iteration or lexical order.
    let ms_val = body.get("milestones")?;
    fn milestone(id: &str, value: &Value) -> Option<Milestone> {
        if id.trim().is_empty() || !value.is_object() {
            return None;
        }
        let requires_evidence = match value.get("requiresEvidence") {
            None => true,
            Some(Value::Bool(value)) => *value,
            Some(value) if value.as_u64() == Some(0) => false,
            Some(value) if value.as_u64() == Some(1) => true,
            _ => return None,
        };
        Some(Milestone {
            identifier: id.into(),
            label: optional_text(value.get("label"))?.unwrap_or_else(|| id.into()),
            description: optional_text(value.get("description"))?,
            acceptance_criteria: optional_text(value.get("acceptanceCriteria"))?,
            maturation_ms: 0,
            requires_evidence,
        })
    }
    let milestones = if let Some(map) = ms_val.as_object() {
        map.iter()
            .map(|(id, value)| milestone(id, value))
            .collect::<Option<Vec<_>>>()?
    } else {
        ms_val
            .as_array()?
            .iter()
            .map(|value| milestone(value.get("identifier")?.as_str()?, value))
            .collect::<Option<Vec<_>>>()?
    };
    if milestones
        .iter()
        .map(|milestone| &milestone.identifier)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != milestones.len()
    {
        return None;
    }
    Some(milestones)
}

fn checked_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Array(chunks) => chunks
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|chunks| chunks.concat()),
        _ => None,
    }
}

fn optional_text(value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None => Some(None),
        Some(value) => Some(Some(checked_string(value)?)),
    }
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
