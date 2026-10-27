use anyhow::{Context, Result};
use std::path::Path;

fn read_document(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut content = String::new();
    std::fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?
        .take(minos::http::MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_string(&mut content)
        .with_context(|| format!("read {}", path.display()))?;
    anyhow::ensure!(
        content.len() <= minos::http::MAX_RESPONSE_BYTES,
        "report exceeds size limit"
    );
    Ok(content)
}
pub(super) struct ReportRelease {
    pub(super) name: String,
    pub(super) date: String,
}

pub(super) struct BackendVersion {
    pub(super) name: String,
    pub(super) pinned: String,
    pub(super) status: String,
}

pub(super) struct ReportPr {
    pub(super) label: String,
    pub(super) url: String,
    pub(super) title: String,
    pub(super) date: Option<String>,
}

pub(super) struct CommunityPr {
    pub(super) contributor: String,
    pub(super) label: String,
    pub(super) url: String,
    pub(super) title: String,
    pub(super) state: String,
    pub(super) response_time: String,
}

pub struct ReportData {
    pub(super) period_label: String,
    pub(super) report_url: String,
    pub(super) releases: Vec<ReportRelease>,
    pub(super) backends: Vec<BackendVersion>,
    pub(super) stats: Vec<(String, String)>,
    pub(super) support_portal: Vec<(String, String)>,
    pub(super) community_prs: Vec<CommunityPr>,
    pub(super) community_stats: Vec<(String, String)>,
    pub(super) prs_merged: Vec<ReportPr>,
    pub(super) prs_in_review: Vec<ReportPr>,
}

// ---------------------------------------------------------------------------
// Report markdown parser
// ---------------------------------------------------------------------------

fn parse_md_link(s: &str) -> Option<(String, String, String)> {
    let s = s.trim_start();
    if !s.starts_with('[') {
        return None;
    }
    let close_bracket = s.find(']')?;
    let label = s[1..close_bracket].to_string();
    let rest = &s[close_bracket + 1..];
    if !rest.starts_with('(') {
        return None;
    }
    let close_paren = rest.find(')')?;
    let url = rest[1..close_paren].to_string();
    let remaining = rest[close_paren + 1..].to_string();
    Some((label, url, remaining))
}

fn parse_kv_lines(body: &str) -> Vec<(String, String)> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            let colon = line.find(':')?;
            let key = line[..colon].trim().to_string();
            let val = line[colon + 1..].trim().to_string();
            if key.is_empty() || val.is_empty() {
                return None;
            }
            Some((key, val))
        })
        .collect()
}

pub fn parse_report_md(path: &Path) -> Result<ReportData> {
    let content = read_document(path)?;
    parse_report_md_str(&content)
}

fn sections(content: &str) -> Result<std::collections::HashMap<String, String>> {
    anyhow::ensure!(
        content.len() <= minos::http::MAX_RESPONSE_BYTES,
        "report exceeds size limit"
    );
    let mut sections: std::collections::HashMap<String, String> = Default::default();
    let mut cur_name = String::new();
    let mut cur_body = String::new();
    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if !cur_name.is_empty() {
                anyhow::ensure!(
                    sections
                        .insert(cur_name.clone(), cur_body.trim().to_string())
                        .is_none(),
                    "duplicate report section: {cur_name}"
                );
            }
            cur_name = heading.trim().to_string();
            anyhow::ensure!(!cur_name.is_empty(), "empty report section heading");
            cur_body = String::new();
        } else {
            cur_body.push_str(line);
            cur_body.push('\n');
        }
    }
    if !cur_name.is_empty() {
        anyhow::ensure!(
            sections
                .insert(cur_name.clone(), cur_body.trim().to_string())
                .is_none(),
            "duplicate report section: {cur_name}"
        );
    }
    Ok(sections)
}

fn nonempty_lines(body: &str) -> impl Iterator<Item = &str> {
    body.lines().map(str::trim).filter(|line| !line.is_empty())
}

fn validate_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url).context("invalid report link URL")?;
    anyhow::ensure!(
        matches!(parsed.scheme(), "http" | "https") && parsed.host_str().is_some(),
        "report links require HTTP(S)"
    );
    Ok(())
}

fn validate_report(sections: &std::collections::HashMap<String, String>) -> Result<()> {
    for name in ["Period", "Stats", "Support Portal"] {
        let mut keys = std::collections::BTreeSet::new();
        for line in nonempty_lines(sections.get(name).map(String::as_str).unwrap_or("")) {
            let (key, value) = line
                .split_once(':')
                .with_context(|| format!("invalid {name} row"))?;
            anyhow::ensure!(
                !key.trim().is_empty() && !value.trim().is_empty() && keys.insert(key.trim()),
                "invalid or duplicate {name} field"
            );
        }
    }
    if let Some(url) = sections.get("Report URL").filter(|url| !url.is_empty()) {
        validate_url(url)?;
    }
    for name in ["Releases", "Backends"] {
        for line in nonempty_lines(sections.get(name).map(String::as_str).unwrap_or("")) {
            let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
            let expected = if name == "Releases" { 2 } else { 4 };
            anyhow::ensure!(
                parts.len() == expected && parts.iter().all(|part| !part.is_empty()),
                "invalid {name} row: {line}"
            );
            if name == "Releases" {
                parts[1]
                    .parse::<chrono::NaiveDate>()
                    .context("invalid release date")?;
            }
        }
    }
    for name in ["PRs Merged", "PRs In Review"] {
        for line in nonempty_lines(sections.get(name).map(String::as_str).unwrap_or("")) {
            if name == "PRs In Review" || line.starts_with('[') {
                let (label, url, remaining) =
                    parse_md_link(line).with_context(|| format!("malformed {name} link"))?;
                anyhow::ensure!(!label.is_empty(), "empty PR link label");
                validate_url(&url)?;
                if name == "PRs Merged" {
                    if let Some((_, date)) = remaining.rsplit_once('|') {
                        date.trim()
                            .parse::<chrono::NaiveDate>()
                            .context("invalid merged PR date")?;
                    }
                }
            }
        }
    }
    for line in nonempty_lines(
        sections
            .get("Community Contributions")
            .map(String::as_str)
            .unwrap_or(""),
    ) {
        if line.contains('|') {
            let parts = line.splitn(4, '|').map(str::trim).collect::<Vec<_>>();
            anyhow::ensure!(
                parts.len() == 4 && parts.iter().all(|part| !part.is_empty()),
                "invalid community contribution row"
            );
            let (_, url, _) = parse_md_link(parts[1]).context("invalid community PR link")?;
            validate_url(&url)?;
        } else {
            let (key, value) = line
                .split_once(':')
                .context("invalid community statistics row")?;
            anyhow::ensure!(
                !key.trim().is_empty() && !value.trim().is_empty(),
                "invalid community statistics field"
            );
        }
    }
    Ok(())
}

pub fn parse_report_md_str(content: &str) -> Result<ReportData> {
    let sections = sections(content)?;
    validate_report(&sections)?;

    // Period → label
    let mut period_start = String::new();
    let mut period_end = String::new();
    for (k, v) in parse_kv_lines(sections.get("Period").map(|s| s.as_str()).unwrap_or("")) {
        match k.as_str() {
            "start" => period_start = v,
            "end" => period_end = v,
            _ => {}
        }
    }
    let start = period_start
        .parse::<chrono::NaiveDate>()
        .context("missing or invalid report period start")?;
    let end = period_end
        .parse::<chrono::NaiveDate>()
        .context("missing or invalid report period end")?;
    anyhow::ensure!(start <= end, "report period starts after it ends");
    let period_label = start.format("%B %Y").to_string();

    let report_url = sections
        .get("Report URL")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();

    // Releases: "Name | YYYY-MM-DD"
    let releases = sections
        .get("Releases")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (name, date) = line.split_once('|')?;
            Some(ReportRelease {
                name: name.trim().to_string(),
                date: date.trim().to_string(),
            })
        })
        .collect();

    // Backends: "name | pinned | latest | status"
    let backends = sections
        .get("Backends")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                return None;
            }
            Some(BackendVersion {
                name: parts[0].trim().to_string(),
                pinned: parts[1].trim().to_string(),
                status: parts[3].trim().to_string(),
            })
        })
        .collect();

    let stats = parse_kv_lines(sections.get("Stats").map(|s| s.as_str()).unwrap_or(""));
    let support_portal = parse_kv_lines(
        sections
            .get("Support Portal")
            .map(|s| s.as_str())
            .unwrap_or(""),
    );

    // Community Contributions: pipe rows + kv stats
    let mut community_prs = Vec::new();
    let mut community_stats = Vec::new();
    for line in sections
        .get("Community Contributions")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line.contains('|') && line.contains("](") {
            // pipe row: contributor | [label](url) title | state | response
            let parts: Vec<&str> = line.splitn(4, '|').collect();
            if parts.len() < 4 {
                continue;
            }
            let contributor = parts[0].trim().to_string();
            let pr_field = parts[1].trim();
            let state = parts[2].trim().to_string();
            let response_time = parts[3].trim().to_string();
            if let Some((label, url, remaining)) = parse_md_link(pr_field) {
                community_prs.push(CommunityPr {
                    contributor,
                    label,
                    url,
                    title: remaining.trim().to_string(),
                    state,
                    response_time,
                });
            }
        } else if line.contains(':') {
            let colon = line.find(':').unwrap();
            let key = line[..colon].trim().to_string();
            let val = line[colon + 1..].trim().to_string();
            if !key.is_empty() && !val.is_empty() {
                community_stats.push((key, val));
            }
        }
    }

    // PRs Merged: "[label](url) title | YYYY-MM-DD" or plain text notes
    let prs_merged = sections
        .get("PRs Merged")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            if let Some((label, url, remaining)) = parse_md_link(line) {
                let remaining = remaining.trim();
                let (title, date) = if let Some(pipe) = remaining.rfind('|') {
                    (
                        remaining[..pipe].trim().to_string(),
                        Some(remaining[pipe + 1..].trim().to_string()),
                    )
                } else {
                    (remaining.to_string(), None)
                };
                Some(ReportPr {
                    label,
                    url,
                    title,
                    date,
                })
            } else {
                Some(ReportPr {
                    label: String::new(),
                    url: String::new(),
                    title: line.to_string(),
                    date: None,
                })
            }
        })
        .collect();

    // PRs In Review: "[label](url) title"
    let prs_in_review = sections
        .get("PRs In Review")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (label, url, remaining) = parse_md_link(line)?;
            Some(ReportPr {
                label,
                url,
                title: remaining.trim().to_string(),
                date: None,
            })
        })
        .collect();

    Ok(ReportData {
        period_label,
        report_url,
        releases,
        backends,
        stats,
        support_portal,
        community_prs,
        community_stats,
        prs_merged,
        prs_in_review,
    })
}

// ---------------------------------------------------------------------------
// Acceptance criteria — report source
// ---------------------------------------------------------------------------

pub struct Signature {
    pub role: String,
    pub name: String,
}

pub struct ProjectConfig {
    pub name: String,
    pub number: String,
    pub signatures: Vec<Signature>,
    pub testing_plan_md: String,
    pub public_comms_md: String,
    pub assurer_comments_md: Option<String>,
}

pub fn parse_project_md(path: &Path) -> Result<ProjectConfig> {
    let content = read_document(path)?;

    let sections = sections(&content)?;

    let mut name = String::new();
    let mut number = String::new();
    for line in sections
        .get("Config")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
    {
        if let Some(v) = line.strip_prefix("name:") {
            name = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("number:") {
            number = v.trim().to_string();
        }
    }

    anyhow::ensure!(
        !name.is_empty() && !number.is_empty(),
        "project Config needs nonempty name and number"
    );
    let mut signatures = Vec::new();
    for line in sections
        .get("Signatures")
        .map(|s| s.as_str())
        .unwrap_or("")
        .lines()
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (role, sig_name) = line
            .split_once('|')
            .context("invalid signature row; expected role | name")?;
        anyhow::ensure!(
            !role.trim().is_empty() && !sig_name.trim().is_empty(),
            "empty signature role/name"
        );
        signatures.push(Signature {
            role: role.trim().to_string(),
            name: sig_name.trim().to_string(),
        });
    }

    Ok(ProjectConfig {
        name,
        number,
        signatures,
        testing_plan_md: sections.get("Testing Plan").cloned().unwrap_or_default(),
        public_comms_md: sections
            .get("Public Communications")
            .cloned()
            .unwrap_or_default(),
        assurer_comments_md: sections
            .get("Third Party Assurer Comments")
            .filter(|comments| !comments.is_empty())
            .cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: &str = "## Period\nstart: 2026-09-01\nend: 2026-09-30\n";

    #[test]
    fn malformed_reports_fail_instead_of_silently_dropping_evidence() {
        for report in [
            "",
            "## Period\nstart: yesterday\nend: tomorrow",
            "## Period\nstart: 2026-10-01\nend: 2026-09-01",
        ] {
            assert!(parse_report_md_str(report).is_err());
        }
        for malformed in [
            "## Period\nstart: 2026-09-02\nend: 2026-09-30",
            "## Releases\nmissing separator",
            "## Releases\nv1 | wrong-date",
            "## Backends\nonly | two",
            "## Stats\nmissing colon",
            "## Support Portal\nkey:",
            "## PRs Merged\n[broken](",
            "## PRs In Review\ninvalid row",
            "## PRs In Review\n[local](file:///etc/hosts)",
            "## Community Contributions\nExample | [broken]( | open",
            "## Report URL\nfile:///etc/hosts",
        ] {
            assert!(
                parse_report_md_str(&format!("{PERIOD}\n{malformed}")).is_err(),
                "{malformed}"
            );
        }
        let valid = parse_report_md_str(&format!("{PERIOD}\n## PRs Merged\nPlain explanatory note\n## PRs In Review\n[#2](https://example.org/pull/2) Review\n")).unwrap();
        assert_eq!(valid.prs_merged.len(), 1);
        assert_eq!(valid.prs_in_review.len(), 1);
    }
}

// ---------------------------------------------------------------------------
// Signature table (separate, not nested in the main form table)
// ---------------------------------------------------------------------------
