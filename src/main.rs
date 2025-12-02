use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc, TimeZone};
use clap::Parser;
use git2::{Repository, Sort};
use std::path::PathBuf;

use charming::{
        Chart, ImageRenderer, ImageFormat,
        component::{Legend, Title,},
        element::{ItemStyle, Label, LabelPosition},
        series::{Pie},
        theme::Theme,
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
        #[arg(short, long, default_value = ".")]
        path: PathBuf,

        #[arg(short, long)]
        email: Vec<String>,

        #[arg(short, long)]
        since: Option<String>,

        #[arg(long, default_value_t = false)]
        partial: bool,

        #[arg(long, default_value_t = false)]
        verbose: bool,
}

fn main() -> Result<()> {
        let args = Args::parse();

        // 1. Parse Date
        let since_date = if let Some(date_str) = &args.since {
                let naive_date = NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
                        .context("Invalid date format. Please use YYYY-MM-DD")?;
                Some(Utc.from_utc_datetime(&naive_date.and_hms_opt(0, 0, 0).unwrap()))
        } else {
                None
        };

        // 2. Open Repo
        let repo = Repository::open(&args.path)
                .with_context(|| format!("Failed to open git repository at {:?}", args.path))?;

        let mut revwalk = repo.revwalk().context("Failed to initialize revision walker")?;
        revwalk.push_head().context("Failed to find HEAD")?;
        revwalk.set_sorting(Sort::TIME)?;

        println!("Scanning repository: {:?}", args.path.canonicalize()?);
        println!("Target Emails:       {}", args.email.join(", "));
        if let Some(d) = since_date {
                println!("Timeframe:           Since {}", d.format("%Y-%m-%d"));
        }
        println!("------------------------------------------------");

        let mut commits_authored = 0;
        let mut total_scanned = 0;

        let mut reviewed_count = 0;
        let mut acked_count = 0;
        let mut tested_count = 0;
        let mut reported_count = 0;

        let search_emails: Vec<String> = args.email.iter().map(|e| e.to_lowercase()).collect();

        for oid in revwalk {
                total_scanned += 1;
                let oid = oid.context("Failed to get object ID")?;
                let commit = repo.find_commit(oid).context("Failed to find commit")?;

                let seconds = commit.time().seconds();
                let commit_time = DateTime::from_timestamp(seconds, 0).unwrap_or_default();

                if let Some(since) = since_date {
                        if commit_time < since { break; }
                }

                let author = commit.author();
                if let Some(author_email) = author.email() {
                        let is_match = if args.partial {
                                search_emails.iter().any(|email| author_email.contains(email))
                        } else {
                                search_emails.iter().any(|email| author_email == email)
                        };
                        if is_match {
                                if args.verbose {
                                        print_commit(&commit, &commit_time);
                                }
                                commits_authored += 1;
                        }
                }

                if let Some(msg) = commit.message() {
                        analyze_trailers(msg, &search_emails, &mut reviewed_count, &mut acked_count, &mut tested_count, &mut reported_count);
                }
        }

        println!("\nSummary:");
        println!("Total Scanned: {}", total_scanned);
        println!("Authored:      {}", commits_authored);
        println!("Reviewed:      {}", reviewed_count);
        println!("Acked:         {}", acked_count);
        println!("Tested:        {}", tested_count);
        println!("Reported:      {}", reported_count);

        println!("Generating Pie Charts...");

        if total_scanned > 0 {
                let total_activity = reviewed_count + acked_count + tested_count +
                        reported_count + commits_authored;
                let no_interaction = if total_scanned > total_activity {
                        total_scanned - total_activity
                } else {
                        0
                };

                let data = vec![
                        ("Authored", commits_authored),
                        ("Reviewed", reviewed_count),
                        ("Acked", acked_count),
                        ("Tested", tested_count),
                        ("Reported", reported_count),
                        ("Non Linaro", no_interaction),
                ];
                if let Some(last_component) = args.path.file_name() {
                        let title = last_component.to_string_lossy().into_owned();
                        let pdate = if let Some(s) = &args.since {
                                format!("{} -- Today", s)
                        } else {
                                "Overall".to_string()
                        };
                        generate_pie_chart(&title, &pdate, data)?;
                }
        }

        Ok(())
}

fn analyze_trailers(msg: &str, targets: &[String], reviewed: &mut i32, acked: &mut i32, tested: &mut i32, reported: &mut i32) {
        for line in msg.lines() {
                let lower = line.trim().to_lowercase();
                if targets.iter().any(|target| lower.contains(target)) {
                        if lower.starts_with("reviewed-by:") { *reviewed += 1; }
                        else if lower.starts_with("acked-by:") { *acked += 1; }
                        else if lower.starts_with("tested-by:") { *tested += 1; }
                        else if lower.starts_with("reported-by:") { *reported += 1; }
                }
        }
}

fn print_commit(commit: &git2::Commit, date: &DateTime<Utc>) {
        let hash = commit.id().to_string();
        let short_hash = &hash[0..7];
        let summary = commit.summary().unwrap_or("No message");
        println!("{} | {} | {}", short_hash, date.format("%Y-%m-%d"), summary);
}

// --- CHARMING (ECharts) GENERATOR ---
fn generate_pie_chart(title: &str, date: &str, data: Vec<(&str, i32)>) -> Result<()> {

        let mut filename = title.to_string();
        filename.push_str(".png");
        // FIX 1: Swap the order. Charming expects (Value, Label), not (Label, Value)
        let pie_data: Vec<(i32, String)> = data.into_iter()
                .map(|(label, value)| (value, label.to_string()))
                .collect();

        // 1. Configure the Title
        //let title = Title::new()
        //.text(title)
        //.subtext(date)
        //.left("center")
        //.text_style(TextStyle::new().font_size(25));

        // SERIES 1: The Percentages (Inside the colored box)
        let inner_series = Pie::new()
                .name(title)
                .radius("70%")
                .data(pie_data.clone()) // Clone data for the first series
                .item_style(ItemStyle::new().border_radius(10).border_color("#fff").border_width(2))
                .label(
                        Label::new()
                        .show(true)
                        .position(LabelPosition::Inside)
                        .formatter("{d}%") // Show only percentage
                        .color("#fff")
                        .font_weight("bold")
                );

        // SERIES 2: The Labels (Outside with pointer lines)
        let outer_series = Pie::new()
                .name(title)
                .radius("70%") // Same radius so it overlaps perfectly
                .data(pie_data)
                .item_style(ItemStyle::new().border_radius(10).border_color("#fff").border_width(2))
                .label(
                        Label::new()
                        .show(true)
                        .position(LabelPosition::Outside)
                        .formatter("{b}") // Show only the Name (e.g., "Authored")
                        .color("#000")
                );

        let chart = Chart::new()
                .legend(Legend::new().top("bottom"))
                .title(
                        Title::new()
                        .text(title)
                        .subtext(date)
                        .left("center"),
                )
                .series(inner_series) // Add Series 1
                .series(outer_series); // Add Series 2

        // Chart dimension 1000x800.
        let mut renderer = ImageRenderer::new(800, 800).theme(Theme::Shine);
        // Render the chart as SVG string.
        renderer.render(&chart).unwrap();
        // Render the chart as PNG bytes.
        renderer.render_format(ImageFormat::Png, &chart).unwrap();
        // Save the chart as SVG file.
        //renderer.save(&chart, filename).unwrap();
        renderer.save_format(ImageFormat::Png, &chart, filename).unwrap();


        Ok(())
}
