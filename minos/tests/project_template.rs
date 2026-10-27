use minos::project_template::clone_project;
use serde_json::json;

#[test]
fn clone_preserves_identity_amounts_and_immediate_first_unlock() {
    let source = json!({"instanceById":{"project":{"identifier":"EXAMPLE-001","label":"Exact project label", "description":"Exact long description", "vendor":{"label":"Example Vendor"},
        "milestones":[
            {"identifier":"m-1","label":"Later", "description":"Later desc", "acceptanceCriteria":"Later criteria", "requiresEvidence":true,"contractMilestoneNumber":2,"value":[{"assetId":"source.token","quantity":"987654321"}]},
            {"identifier":"m-0","label":"M0", "description":"", "acceptanceCriteria":"Signed contract", "requiresEvidence":false,"contractMilestoneNumber":1,"value":[{"assetId":"source.token","quantity":"123456789"}]}]}}});
    let template = clone_project(
        &source,
        15,
        1_000_000,
        Some("ab".repeat(28)),
        Some("0014df105553444d".into()),
    )
    .unwrap();
    assert_eq!(template.project, "EXAMPLE-001");
    assert_eq!(template.label, "Exact project label");
    assert_eq!(template.description, "Exact long description");
    assert_eq!(template.milestones[0].id, "m-0");
    assert_eq!(template.milestones[0].maturation_ms, 1_000_000);
    assert_eq!(template.milestones[1].maturation_ms, 1_900_000);
    assert_eq!(template.milestones[0].token_amount, 123456789);
    assert_eq!(template.milestones[1].token_amount, 987654321);
    assert_eq!(
        template.milestones[0].acceptance_criteria.as_deref(),
        Some("Signed contract")
    );
    assert!(!template.milestones[0].requires_evidence);
    assert!(clone_project(&source, 15, 1_000_000, None, None).is_err());
    assert!(clone_project(
        &source,
        0,
        1_000_000,
        Some("ab".repeat(28)),
        Some("00".into())
    )
    .is_err());
}

#[test]
fn fixed_ada_clone_replaces_all_amounts_but_preserves_metadata_and_schedule() {
    let source = json!({"instanceById":{"project":{"identifier":"EXAMPLE-001","label":"Same name","description":"Same description","vendor":{"label":"Vendor"},"milestones":[
        {"identifier":"m-1","contractMilestoneNumber":2,"label":"Later","description":"Later description","acceptanceCriteria":"Later criteria","requiresEvidence":true,"value":[{"assetId":"ada.lovelace","quantity":"1189560"},{"assetId":"source.token","quantity":"999999999999"}]},
        {"identifier":"m-0","contractMilestoneNumber":1,"label":"Initial","description":"Initial description","acceptanceCriteria":"Signed contract","requiresEvidence":false,"value":[{"assetId":"source.token","quantity":"123"}]}]}}});
    let template = minos::project_template::clone_project_ada(&source, 15, 1_000_000, 50).unwrap();
    assert_eq!(template.label, "Same name");
    assert_eq!(template.description, "Same description");
    assert!(template.token_policy.is_none() && template.token_name.is_none());
    assert!(template
        .milestones
        .iter()
        .all(|m| m.lovelace == 50_000_000 && m.token_amount == 0));
    assert_eq!(template.milestones[0].id, "m-0");
    assert_eq!(template.milestones[0].maturation_ms, 1_000_000);
    assert_eq!(template.milestones[1].maturation_ms, 1_900_000);
    assert_eq!(
        template.milestones[0].acceptance_criteria.as_deref(),
        Some("Signed contract")
    );
    assert!(!template.milestones[0].requires_evidence);
    assert!(template.milestones[1].requires_evidence);
    assert!(minos::project_template::clone_project_ada(&source, 15, 1_000_000, 0).is_err());
    assert!(minos::project_template::clone_project_ada(&source, 15, 1_000_000, u64::MAX).is_err());
}
