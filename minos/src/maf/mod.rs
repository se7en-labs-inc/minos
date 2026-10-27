mod fetch;
mod parse;
mod render;
use anyhow::{Context, Result};
use chrono::NaiveDate;
use fetch::*;
use parse::*;
use render::*;
pub async fn cmd_maf_github(
    repos: Vec<String>,
    start_date: NaiveDate,
    end_date: NaiveDate,
    milestone: String,
    project_path: std::path::PathBuf,
    out: std::path::PathBuf,
) -> Result<()> {
    anyhow::ensure!(!repos.is_empty(), "provide at least one repository");
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
    let assurer_comments = project
        .assurer_comments_md
        .as_deref()
        .map(|md| markdown_to_paras(&sub(md)));

    let opts = MafOptions {
        project_name: project.name,
        project_number: project.number,
        milestone,
        signatures: project.signatures,
        testing_plan,
        public_comms,
        assurer_comments,
    };

    let ac = build_acceptance_paras(&all_prs, &all_rels, start_date, end_date);
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
        let response = gh_client()?
            .get(&report_source)
            .send()
            .await
            .with_context(|| format!("fetch {report_source}"))?
            .error_for_status()
            .with_context(|| format!("HTTP error fetching {report_source}"))?;
        let body = minos::http::text(response)
            .await
            .context("read report response body")?;
        parse_report_md_str(&body)?
    } else {
        parse_report_md(std::path::Path::new(&report_source))?
    };
    let month_year = report.period_label.clone();
    let sub = |s: &str| s.replace("[MONTH YEAR]", &month_year);
    let testing_plan = markdown_to_paras(&sub(&project.testing_plan_md));
    let public_comms = markdown_to_paras(&sub(&project.public_comms_md));
    let assurer_comments = project
        .assurer_comments_md
        .as_deref()
        .map(|md| markdown_to_paras(&sub(md)));
    let ac = build_acceptance_paras_from_report(&report);
    let opts = MafOptions {
        project_name: project.name,
        project_number: project.number,
        milestone,
        signatures: project.signatures,
        testing_plan,
        public_comms,
        assurer_comments,
    };
    generate_docx(&opts, ac, &out)?;
    println!("Written: {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::path::Path;

    #[test]
    fn report_and_project_produce_expected_document_and_write_errors_propagate() {
        let report = parse_report_md_str("## Period\nstart: 2026-09-01\nend: 2026-09-30\n\n## Releases\nv1.0 | 2026-09-15\n\n## PRs Merged\n[#101](https://example.org/pull/101) Preserve funds | 2026-09-16\n").unwrap();
        assert_eq!(report.period_label, "September 2026");
        assert_eq!(report.prs_merged.len(), 1);
        let config = parse_project_md(Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/maf/project.md"
        )))
        .unwrap();
        let options = MafOptions {
            project_name: config.name,
            project_number: config.number,
            milestone: "M1 – Synthetic acceptance".into(),
            signatures: config.signatures,
            testing_plan: markdown_to_paras(&config.testing_plan_md),
            public_comms: markdown_to_paras(&config.public_comms_md),
            assurer_comments: config.assurer_comments_md.as_deref().map(markdown_to_paras),
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("minos-maf-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let output = root.join("form.docx");
        generate_docx(
            &options,
            build_acceptance_paras_from_report(&report),
            &output,
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
        let mut document = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut document)
            .unwrap();
        for text in [
            "Example Treasury Tooling",
            "EXAMPLE-001",
            "Synthetic acceptance",
            "Preserve funds",
            "Example Contributor",
            "Example Reviewer",
            "Review is pending",
        ] {
            assert!(document.contains(text), "missing document content: {text}");
        }
        let mut relationships = String::new();
        archive
            .by_name("word/_rels/document.xml.rels")
            .unwrap()
            .read_to_string(&mut relationships)
            .unwrap();
        assert!(relationships.contains("https://example.org/pull/101"));
        assert!(generate_docx(&options, vec![], &root.join("missing/form.docx")).is_err());
        drop(archive);
        let all_prs = vec![(
            "example/project".into(),
            (0..101)
                .map(|number| PrItem {
                    number,
                    title: format!("Paginated change {number}"),
                    html_url: format!("https://example.org/pr/{number}"),
                })
                .collect(),
        )];
        let all_releases = vec![(
            "example/project".into(),
            (0..101)
                .map(|number| Release {
                    tag_name: format!("version-{number}"),
                    name: None,
                    html_url: format!("https://example.org/releases/{number}"),
                    published_at: Some("2026-09-15T00:00:00Z".into()),
                })
                .collect(),
        )];
        let output = root.join("paginated.docx");
        generate_docx(
            &options,
            build_acceptance_paras(
                &all_prs,
                &all_releases,
                "2026-09-01".parse().unwrap(),
                "2026-09-30".parse().unwrap(),
            ),
            &output,
        )
        .unwrap();
        let mut archive = zip::ZipArchive::new(std::fs::File::open(output).unwrap()).unwrap();
        let mut document = String::new();
        archive
            .by_name("word/document.xml")
            .unwrap()
            .read_to_string(&mut document)
            .unwrap();
        for text in [
            "Paginated change 100",
            "version-100",
            "All requested pages were retrieved",
            "2026-09-01 through 2026-09-30",
        ] {
            assert!(
                document.contains(text),
                "missing paginated report content: {text}"
            );
        }
        drop(archive);
        std::fs::remove_dir_all(root).unwrap();
    }
}
