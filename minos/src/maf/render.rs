use super::fetch::{PrItem, Release};
use super::parse::{ReportData, Signature};
use anyhow::{Context, Result};
use docx_rs::*;
use std::path::Path;
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

pub(super) fn build_acceptance_paras_from_report(report: &ReportData) -> Vec<Paragraph> {
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
        // Published Daedalus report data can still reference the artifact host.
        // Point readers to the public reports page while retaining other projects' URLs.
        let report_url = if report
            .report_url
            .starts_with("https://daedalus.se7enlabs.org/reports/")
        {
            "https://daedaluswallet.io/reports/"
        } else {
            &report.report_url
        };
        out.push(p_link("Monthly reports: ", report_url, report_url));
    }

    out
}

// ---------------------------------------------------------------------------
// Acceptance criteria — GitHub source
// ---------------------------------------------------------------------------

pub(super) fn build_acceptance_paras(
    all_prs: &[(String, Vec<PrItem>)],
    all_rels: &[(String, Vec<Release>)],
    start: chrono::NaiveDate,
    end: chrono::NaiveDate,
) -> Vec<Paragraph> {
    let repositories = all_prs
        .iter()
        .map(|(repo, _)| repo.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let scope = p(&format!("Sources: GitHub API for {repositories}, {start} through {end} inclusive. All requested pages were retrieved. PRs are filtered by merge date and releases by publication date. Unpublished releases and Git tags without release records are excluded. Activity remains subject to milestone acceptance review."));
    let has_prs = all_prs.iter().any(|(_, v)| !v.is_empty());
    let has_rels = all_rels.iter().any(|(_, v)| !v.is_empty());

    if !has_prs && !has_rels {
        return vec![
            scope,
            p("No merged pull requests or releases found for this period."),
        ];
    }

    let mut out: Vec<Paragraph> = vec![scope, Paragraph::new()
        .add_run(rb("Source Activity: "))
        .add_run(r(
            "The following merged pull requests and published releases were retrieved for the specified period.",
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
        out.push(pb(&format!("GitHub Release Records — {repo}")));
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
    pub assurer_comments: Option<Vec<Paragraph>>,
}

pub fn generate_docx(
    opts: &MafOptions,
    acceptance_paras: Vec<Paragraph>,
    out: &Path,
) -> Result<()> {
    let ac = acceptance_paras;
    let mut comments_cell = TableCell::new()
        .width(TOTAL_W, WidthType::Dxa)
        .grid_span(2)
        .shading(Shading::new().fill("FFFFFF").color("auto"));
    let comments = opts
        .assurer_comments
        .clone()
        .unwrap_or_else(|| vec![p(""); 5]);
    for paragraph in comments {
        comments_cell = comments_cell.add_paragraph(paragraph);
    }

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
        TableRow::new(vec![comments_cell]),
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
