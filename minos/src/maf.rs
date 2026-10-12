use anyhow::{Context, Result};
use chrono::NaiveDate;
use docx_rs::*;
use serde::Deserialize;
use std::path::Path;

// ---------------------------------------------------------------------------
// GitHub types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct SearchResponse {
    items: Vec<PrItem>,
}

#[derive(Debug, Deserialize)]
pub struct PrItem {
    pub number: u64,
    pub title: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub name: Option<String>,
    pub html_url: String,
    pub published_at: Option<String>,
}

impl Release {
    fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.tag_name)
    }
    fn date_str(&self) -> &str {
        self.published_at
            .as_deref()
            .and_then(|s| s.get(..10))
            .unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// GitHub fetching
// ---------------------------------------------------------------------------

fn gh_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("minos-maf/0.1")
        .build()
        .context("build HTTP client")
}

pub async fn fetch_merged_prs(repo: &str, start: NaiveDate, end: NaiveDate) -> Result<Vec<PrItem>> {
    let client = gh_client()?;
    let q = format!("repo:{repo} is:pr is:merged merged:{start}..{end}");
    let resp: SearchResponse = client
        .get("https://api.github.com/search/issues")
        .query(&[
            ("q", q.as_str()),
            ("per_page", "100"),
            ("sort", "created"),
            ("order", "asc"),
        ])
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("GitHub search request")?
        .error_for_status()
        .context("GitHub search status")?
        .json()
        .await
        .context("parse GitHub search response")?;
    Ok(resp.items)
}

pub async fn fetch_releases(repo: &str, start: NaiveDate, end: NaiveDate) -> Result<Vec<Release>> {
    let client = gh_client()?;
    let all: Vec<Release> = client
        .get(format!("https://api.github.com/repos/{repo}/releases"))
        .query(&[("per_page", "100")])
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .context("GitHub releases request")?
        .error_for_status()
        .context("GitHub releases status")?
        .json()
        .await
        .context("parse GitHub releases response")?;
    Ok(all
        .into_iter()
        .filter(|r| {
            r.published_at
                .as_deref()
                .and_then(|s| s.get(..10))
                .and_then(|s| s.parse::<NaiveDate>().ok())
                .map(|d| d >= start && d <= end)
                .unwrap_or(false)
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Layout constants  (letter paper, 1" margins → 9360 twips usable)
// ---------------------------------------------------------------------------

const LABEL_W: usize = 1980;
const CONTENT_W: usize = 7380;
const TOTAL_W: usize = LABEL_W + CONTENT_W; // 9360

// Colours
const BRAND: &str = "1A3A5C"; // dark navy — title / section headers
const BRAND_FG: &str = "FFFFFF";
const LABEL_BG: &str = "E8F0F7"; // very light blue for label cells
const LINK: &str = "1155CC";

// Typography
const FONT: &str = "Calibri";
const PT10: usize = 20; // half-points
const PT11: usize = 22;
const PT14: usize = 28;
const PT18: usize = 36;

// Cell padding (twips)
const PAD: usize = 100;

// ---------------------------------------------------------------------------
// Run / paragraph helpers
// ---------------------------------------------------------------------------

fn r(s: &str) -> Run {
    Run::new()
        .add_text(s)
        .size(PT10)
        .fonts(RunFonts::new().ascii(FONT).hi_ansi(FONT))
}

fn rb(s: &str) -> Run {
    r(s).bold()
}

fn rc(s: &str, color: &str) -> Run {
    r(s).color(color)
}

fn p(s: &str) -> Paragraph {
    Paragraph::new()
        .add_run(r(s))
        .line_spacing(LineSpacing::new().after(60))
}

fn pb(s: &str) -> Paragraph {
    Paragraph::new()
        .add_run(rb(s))
        .line_spacing(LineSpacing::new().after(60))
}

fn gap() -> Paragraph {
    Paragraph::new()
        .add_run(Run::new().size(PT10))
        .line_spacing(LineSpacing::new().after(0).before(0))
}

fn link_run(label: &str) -> Run {
    Run::new()
        .add_text(label)
        .size(PT10)
        .color(LINK)
        .underline("single")
        .fonts(RunFonts::new().ascii(FONT).hi_ansi(FONT))
}

fn p_link(prefix: &str, label: &str, url: &str) -> Paragraph {
    let mut para = Paragraph::new().line_spacing(LineSpacing::new().after(40));
    if !prefix.is_empty() {
        para = para.add_run(r(prefix));
    }
    para.add_hyperlink(Hyperlink::new(url, HyperlinkType::External).add_run(link_run(label)))
}

// ---------------------------------------------------------------------------
// Markdown → paragraphs
// ---------------------------------------------------------------------------

pub fn markdown_to_paras(md: &str) -> Vec<Paragraph> {
    let mut out = Vec::new();
    for block in md.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let text = block.trim_start_matches('#').trim_start_matches(' ');
        let mut para = Paragraph::new().line_spacing(LineSpacing::new().after(80));
        let mut rem = text;
        while !rem.is_empty() {
            if let Some(s) = rem.find("**") {
                if s > 0 {
                    para = para.add_run(r(&rem[..s]));
                }
                rem = &rem[s + 2..];
                if let Some(e) = rem.find("**") {
                    para = para.add_run(rb(&rem[..e]));
                    rem = &rem[e + 2..];
                } else {
                    para = para.add_run(r(rem));
                    rem = "";
                }
            } else {
                para = para.add_run(r(rem));
                rem = "";
            }
        }
        out.push(para);
    }
    if out.is_empty() {
        out.push(p("[To be completed]"));
    }
    out
}

// ---------------------------------------------------------------------------
// Cell builders
// ---------------------------------------------------------------------------

fn cell_padded(w: usize, paragraphs: Vec<Paragraph>) -> TableCell {
    let mut cell = TableCell::new().width(w, WidthType::Dxa);
    for pg in paragraphs {
        cell = cell.add_paragraph(pg);
    }
    cell
}

fn label_cell(s: &str) -> TableCell {
    cell_padded(
        LABEL_W,
        vec![Paragraph::new().add_run(
            Run::new()
                .add_text(s)
                .size(PT10)
                .bold()
                .color(BRAND)
                .fonts(RunFonts::new().ascii(FONT).hi_ansi(FONT)),
        )],
    )
    .shading(Shading::new().fill(LABEL_BG).color("auto"))
}

fn content_cell(paragraphs: Vec<Paragraph>) -> TableCell {
    cell_padded(CONTENT_W, paragraphs)
}

fn form_row(label: &str, paragraphs: Vec<Paragraph>) -> TableRow {
    TableRow::new(vec![label_cell(label), content_cell(paragraphs)])
}

fn single_row(label: &str, value: &str) -> TableRow {
    form_row(label, vec![p(value)])
}

// Short white spacer row between sections
fn divider_row() -> TableRow {
    TableRow::new(vec![TableCell::new()
        .width(TOTAL_W, WidthType::Dxa)
        .grid_span(2)
        .shading(Shading::new().fill("FFFFFF").color("auto"))])
    .row_height(120.0)
    .height_rule(HeightRule::Exact)
}

// A full-width banner row spanning both columns
fn banner_row(text: &str, font_size: usize, bg: &str, fg: &str) -> TableRow {
    TableRow::new(vec![TableCell::new()
        .width(TOTAL_W, WidthType::Dxa)
        .grid_span(2)
        .shading(Shading::new().fill(bg).color("auto"))
        .add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(text)
                    .size(font_size)
                    .bold()
                    .color(fg)
                    .fonts(RunFonts::new().ascii(FONT).hi_ansi(FONT)),
            ),
        )])
}

fn title_row(text: &str) -> TableRow {
    banner_row(text, PT18, BRAND, BRAND_FG)
}

fn section_row(text: &str) -> TableRow {
    banner_row(text, PT11, BRAND, BRAND_FG)
}

// ---------------------------------------------------------------------------
// Report data types
// ---------------------------------------------------------------------------

struct ReportRelease {
    name: String,
    date: String,
}

struct BackendVersion {
    name: String,
    pinned: String,
    latest: String,
    status: String,
}

struct ReportPr {
    label: String,
    url: String,
    title: String,
    date: Option<String>,
}

struct CommunityPr {
    contributor: String,
    label: String,
    url: String,
    title: String,
    state: String,
    response_time: String,
}

pub struct ReportData {
    period_label: String,
    report_url: String,
    releases: Vec<ReportRelease>,
    backends: Vec<BackendVersion>,
    stats: Vec<(String, String)>,
    support_portal: Vec<(String, String)>,
    community_prs: Vec<CommunityPr>,
    community_stats: Vec<(String, String)>,
    prs_merged: Vec<ReportPr>,
    prs_in_review: Vec<ReportPr>,
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
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    parse_report_md_str(&content)
}

pub fn parse_report_md_str(content: &str) -> Result<ReportData> {
    let mut sections: std::collections::HashMap<String, String> = Default::default();
    let mut cur_name = String::new();
    let mut cur_body = String::new();
    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if !cur_name.is_empty() {
                sections.insert(cur_name.clone(), cur_body.trim().to_string());
            }
            cur_name = heading.trim().to_string();
            cur_body = String::new();
        } else {
            cur_body.push_str(line);
            cur_body.push('\n');
        }
    }
    if !cur_name.is_empty() {
        sections.insert(cur_name, cur_body.trim().to_string());
    }

    // Period → label
    let mut period_label = String::new();
    let mut period_start = String::new();
    let mut period_end = String::new();
    for (k, v) in parse_kv_lines(sections.get("Period").map(|s| s.as_str()).unwrap_or("")) {
        match k.as_str() {
            "start" => period_start = v,
            "end" => period_end = v,
            _ => {}
        }
    }
    if !period_start.is_empty() {
        // "2026-09-01" → "September 2026"
        if let Ok(d) = period_start.parse::<chrono::NaiveDate>() {
            period_label = d.format("%B %Y").to_string();
        }
        let _ = period_end;
    }

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
                latest: parts[2].trim().to_string(),
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

fn pr_para(label: &str, url: &str, title: &str, suffix: Option<&str>) -> Paragraph {
    let mut para = Paragraph::new().line_spacing(LineSpacing::new().after(40));
    if url.is_empty() {
        para = para.add_run(r(title));
    } else {
        para = para.add_run(r("  "));
        para = para
            .add_hyperlink(Hyperlink::new(url, HyperlinkType::External).add_run(link_run(label)));
        if !title.is_empty() {
            para = para.add_run(r(&format!(" {title}")));
        }
    }
    if let Some(s) = suffix {
        para = para.add_run(rc(&format!(" — {s}"), "555555"));
    }
    para
}

fn build_acceptance_paras_from_report(report: &ReportData) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();

    // Releases
    if !report.releases.is_empty() {
        let list = report
            .releases
            .iter()
            .map(|r| format!("{} ({})", r.name, r.date))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(
            Paragraph::new()
                .add_run(rb("Releases shipped this period: "))
                .add_run(r(&format!("{list}.")))
                .line_spacing(LineSpacing::new().after(60)),
        );
    }

    // Backends
    if !report.backends.is_empty() {
        let list = report
            .backends
            .iter()
            .map(|b| format!("{} {} ({})", b.name, b.pinned, b.status.to_lowercase()))
            .collect::<Vec<_>>()
            .join("; ");
        out.push(
            Paragraph::new()
                .add_run(rb("Backends pinned at month end: "))
                .add_run(r(&format!("{list}.")))
                .line_spacing(LineSpacing::new().after(60)),
        );
    }

    // Stats summary
    if !report.stats.is_empty() {
        let line = report
            .stats
            .iter()
            .map(|(k, v)| format!("{v} {}", k.to_lowercase()))
            .collect::<Vec<_>>()
            .join(", ");
        out.push(p(&line));
    }

    out.push(gap());

    // Merged PRs
    if !report.prs_merged.is_empty() {
        out.push(pb("Merged"));
        for pr in &report.prs_merged {
            out.push(pr_para(&pr.label, &pr.url, &pr.title, pr.date.as_deref()));
        }
        out.push(gap());
    }

    // In review PRs
    if !report.prs_in_review.is_empty() {
        out.push(pb("In Review"));
        for pr in &report.prs_in_review {
            out.push(pr_para(&pr.label, &pr.url, &pr.title, None));
        }
        out.push(gap());
    }

    // Community contributions
    if !report.community_prs.is_empty() || !report.community_stats.is_empty() {
        out.push(pb("Community Contributions"));
        if !report.community_stats.is_empty() {
            let line = report
                .community_stats
                .iter()
                .map(|(k, v)| format!("{k}: {v}"))
                .collect::<Vec<_>>()
                .join("; ");
            out.push(p(&line));
        }
        for pr in &report.community_prs {
            let suffix = format!("{}, {}", pr.state, pr.response_time);
            let mut para = Paragraph::new().line_spacing(LineSpacing::new().after(40));
            para = para.add_run(r(&format!("  {} — ", pr.contributor)));
            para = para.add_hyperlink(
                Hyperlink::new(&pr.url, HyperlinkType::External).add_run(link_run(&pr.label)),
            );
            if !pr.title.is_empty() {
                para = para.add_run(r(&format!(" {}", pr.title)));
            }
            para = para.add_run(rc(&format!(" ({suffix})"), "555555"));
            out.push(para);
        }
        out.push(gap());
    }

    // Support portal
    if !report.support_portal.is_empty() {
        out.push(pb("User Support Portal"));
        let line = report
            .support_portal
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("; ");
        out.push(p(&line));
        out.push(gap());
    }

    // Report URL
    if !report.report_url.is_empty() {
        out.push(p_link(
            "Full monthly report: ",
            &report.report_url,
            &report.report_url,
        ));
    }

    out
}

// ---------------------------------------------------------------------------
// Acceptance criteria — GitHub source
// ---------------------------------------------------------------------------

fn build_acceptance_paras(
    all_prs: &[(String, Vec<PrItem>)],
    all_rels: &[(String, Vec<Release>)],
) -> Vec<Paragraph> {
    let has_prs = all_prs.iter().any(|(_, v)| !v.is_empty());
    let has_rels = all_rels.iter().any(|(_, v)| !v.is_empty());

    if !has_prs && !has_rels {
        return vec![p(
            "No merged pull requests or releases found for this period.",
        )];
    }

    let mut out: Vec<Paragraph> = vec![Paragraph::new()
        .add_run(rb("Completion Statement: "))
        .add_run(r(
            "Maintenance and improvement work has been completed for the specified period.",
        ))
        .line_spacing(LineSpacing::new().after(120))];

    for (repo, prs) in all_prs {
        if prs.is_empty() {
            continue;
        }
        out.push(pb(&format!("Pull Requests Merged — {repo}")));
        for pr in prs {
            out.push(p_link(
                &format!("  PR #{}: ", pr.number),
                &pr.title,
                &pr.html_url,
            ));
        }
        out.push(gap());
    }

    for (repo, rels) in all_rels {
        if rels.is_empty() {
            continue;
        }
        out.push(pb(&format!("Releases Published — {repo}")));
        for rel in rels {
            out.push(p_link(
                &format!("  {} — {} — ", rel.tag_name, rel.date_str()),
                rel.display_name(),
                &rel.html_url,
            ));
        }
        out.push(gap());
    }

    out
}

// ---------------------------------------------------------------------------
// Project config (parsed from project.md)
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
}

pub fn parse_project_md(path: &Path) -> Result<ProjectConfig> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;

    // Split into named sections at "## Heading" lines
    let mut sections: std::collections::HashMap<String, String> = Default::default();
    let mut cur_name = String::new();
    let mut cur_body = String::new();

    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if !cur_name.is_empty() {
                sections.insert(cur_name.clone(), cur_body.trim().to_string());
            }
            cur_name = heading.trim().to_string();
            cur_body = String::new();
        } else {
            cur_body.push_str(line);
            cur_body.push('\n');
        }
    }
    if !cur_name.is_empty() {
        sections.insert(cur_name, cur_body.trim().to_string());
    }

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
        if let Some((role, sig_name)) = line.split_once('|') {
            signatures.push(Signature {
                role: role.trim().to_string(),
                name: sig_name.trim().to_string(),
            });
        }
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
    })
}

// ---------------------------------------------------------------------------
// Signature table (separate, not nested in the main form table)
// ---------------------------------------------------------------------------

fn sig_header_cell(s: &str, w: usize) -> TableCell {
    TableCell::new()
        .width(w, WidthType::Dxa)
        .shading(Shading::new().fill(BRAND).color("auto"))
        .add_paragraph(
            Paragraph::new().add_run(
                Run::new()
                    .add_text(s)
                    .size(PT10)
                    .bold()
                    .color(BRAND_FG)
                    .fonts(RunFonts::new().ascii(FONT).hi_ansi(FONT)),
            ),
        )
}

fn bordered_cell(s: &str, w: usize) -> TableCell {
    let border = || {
        [
            TableCellBorderPosition::Top,
            TableCellBorderPosition::Bottom,
            TableCellBorderPosition::Left,
            TableCellBorderPosition::Right,
        ]
        .into_iter()
        .map(|pos| TableCellBorder::new(pos).color(BRAND).size(4))
    };
    let mut cell = cell_padded(w, vec![p(s)]);
    for b in border() {
        cell = cell.set_border(b);
    }
    cell
}

fn make_sig_table(signatures: &[Signature]) -> Table {
    // role 2200, name 2800, sig 2800, date 1560 → total 9360
    let ws = [2200usize, 2800, 2800, 1560];
    let headers = ["Role", "Name", "Signature", "Date of Signature"];
    let header_row = TableRow::new(
        headers
            .iter()
            .zip(ws.iter())
            .map(|(h, w)| sig_header_cell(h, *w))
            .collect(),
    );
    let mut rows = vec![header_row];
    for sig in signatures {
        rows.push(TableRow::new(
            [sig.role.as_str(), sig.name.as_str(), "", ""]
                .iter()
                .zip(ws.iter())
                .map(|(v, w)| bordered_cell(v, *w))
                .collect(),
        ));
    }
    Table::new(rows)
        .set_grid(ws.to_vec())
        .width(TOTAL_W, WidthType::Dxa)
        .layout(TableLayoutType::Fixed)
        .margins(TableCellMargins::new().margin(PAD, PAD, PAD, PAD))
}

// ---------------------------------------------------------------------------
// Document entry point
// ---------------------------------------------------------------------------

pub struct MafOptions {
    pub project_name: String,
    pub project_number: String,
    pub milestone: String,
    pub signatures: Vec<Signature>,
    pub testing_plan: Vec<Paragraph>,
    pub public_comms: Vec<Paragraph>,
}

pub fn generate_docx(
    opts: &MafOptions,
    acceptance_paras: Vec<Paragraph>,
    out: &Path,
) -> Result<()> {
    let ac = acceptance_paras;

    let main_table = Table::new(vec![
        title_row("Milestone Acceptance Form (MAF)"),
        section_row("Project Details"),
        single_row("Project Name", &opts.project_name),
        single_row("Project Number", &opts.project_number),
        divider_row(),
        section_row("Milestone"),
        single_row("Milestone Name(s) & Number(s)", &opts.milestone),
        divider_row(),
        section_row("Evidence"),
        form_row("Acceptance Criteria Met", ac),
        divider_row(),
        form_row("Quality & Testing", opts.testing_plan.clone()),
        divider_row(),
        form_row("Public Communication Summary", opts.public_comms.clone()),
        divider_row(),
        section_row("Third Party Assurer Comments"),
        TableRow::new(vec![TableCell::new()
            .width(TOTAL_W, WidthType::Dxa)
            .grid_span(2)
            .shading(Shading::new().fill("FFFFFF").color("auto"))
            .add_paragraph(p(""))
            .add_paragraph(p(""))
            .add_paragraph(p(""))
            .add_paragraph(p(""))
            .add_paragraph(p(""))]),
        divider_row(),
        section_row("Signature Authorization"),
    ])
    .set_grid(vec![LABEL_W, CONTENT_W])
    .width(TOTAL_W, WidthType::Dxa)
    .layout(TableLayoutType::Fixed)
    .margins(TableCellMargins::new().margin(PAD, PAD, PAD, PAD));

    let file = std::fs::File::create(out).with_context(|| format!("create {}", out.display()))?;

    Docx::new()
        .add_table(main_table)
        .add_paragraph(gap())
        .add_table(make_sig_table(&opts.signatures))
        .build()
        .pack(file)
        .map_err(|e| anyhow::anyhow!("write docx: {e:?}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Command entry point
// ---------------------------------------------------------------------------

pub async fn cmd_maf_github(
    repos: Vec<String>,
    start_date: NaiveDate,
    end_date: NaiveDate,
    milestone: String,
    project_path: std::path::PathBuf,
    out: std::path::PathBuf,
) -> Result<()> {
    let project = parse_project_md(&project_path)?;
    let month_year = start_date.format("%B %Y").to_string();

    let mut all_prs: Vec<(String, Vec<PrItem>)> = Vec::new();
    let mut all_rels: Vec<(String, Vec<Release>)> = Vec::new();

    for repo in &repos {
        println!("Fetching PRs for {repo}...");
        let prs = fetch_merged_prs(repo, start_date, end_date)
            .await
            .with_context(|| format!("fetch PRs for {repo}"))?;
        println!("  {} merged PR(s)", prs.len());

        println!("Fetching releases for {repo}...");
        let rels = fetch_releases(repo, start_date, end_date)
            .await
            .with_context(|| format!("fetch releases for {repo}"))?;
        println!("  {} release(s)", rels.len());

        all_prs.push((repo.clone(), prs));
        all_rels.push((repo.clone(), rels));
    }

    let sub = |s: &str| s.replace("[MONTH YEAR]", &month_year);
    let testing_plan = markdown_to_paras(&sub(&project.testing_plan_md));
    let public_comms = markdown_to_paras(&sub(&project.public_comms_md));

    let opts = MafOptions {
        project_name: project.name,
        project_number: project.number,
        milestone,
        signatures: project.signatures,
        testing_plan,
        public_comms,
    };

    let ac = build_acceptance_paras(&all_prs, &all_rels);
    generate_docx(&opts, ac, &out)?;
    println!("Written: {}", out.display());
    Ok(())
}

pub async fn cmd_maf_report(
    report_source: String,
    milestone: String,
    project_path: std::path::PathBuf,
    out: std::path::PathBuf,
) -> Result<()> {
    let project = parse_project_md(&project_path)?;
    let report = if report_source.starts_with("http://") || report_source.starts_with("https://") {
        let body = gh_client()?
            .get(&report_source)
            .send()
            .await
            .with_context(|| format!("fetch {report_source}"))?
            .error_for_status()
            .with_context(|| format!("HTTP error fetching {report_source}"))?
            .text()
            .await
            .context("read response body")?;
        parse_report_md_str(&body)?
    } else {
        parse_report_md(std::path::Path::new(&report_source))?
    };
    let month_year = report.period_label.clone();
    let sub = |s: &str| s.replace("[MONTH YEAR]", &month_year);
    let testing_plan = markdown_to_paras(&sub(&project.testing_plan_md));
    let public_comms = markdown_to_paras(&sub(&project.public_comms_md));
    let ac = build_acceptance_paras_from_report(&report);
    let opts = MafOptions {
        project_name: project.name,
        project_number: project.number,
        milestone,
        signatures: project.signatures,
        testing_plan,
        public_comms,
    };
    generate_docx(&opts, ac, &out)?;
    println!("Written: {}", out.display());
    Ok(())
}
