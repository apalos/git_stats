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
        email: String,

        #[arg(short, long)]
        since: Option<String>,

        #[arg(long, default_value_t = false)]
        partial: bool,

        #[arg(long, default_value_t = false)]
        verbose: bool,
}

#[derive(Default, Debug)]
struct TrailerState {
    signed_off: bool,
    reviewed: bool,
    acked: bool,
    tested: bool,
    reported: bool,
}

impl TrailerState {
    // Helper to check if any flag has been latched
    fn any_active(&self) -> bool {
        self.signed_off || self.reviewed || self.acked || self.tested || self.reported
    }
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
        println!("Target Email:        {}", args.email);
        if let Some(d) = since_date {
                println!("Timeframe:           Since {}", d.format("%Y-%m-%d"));
        }
        println!("------------------------------------------------");

        // The totals
        let mut commits_authored = 0;
        let mut commits_touched = 0;
        let mut commits_ignored = 0;
        let mut total_scanned = 0;

        // Additional details
        let mut signed_off_count = 0;
        let mut reviewed_count = 0;
        let mut acked_count = 0;
        let mut tested_count = 0;
        let mut reported_count = 0;

        let search_email = args.email.to_lowercase();

        for oid in revwalk {
                let oid = oid.context("Failed to get object ID")?;
                let commit = repo.find_commit(oid).context("Failed to find commit")?;

                let seconds = commit.time().seconds();
                let commit_time = DateTime::from_timestamp(seconds, 0).unwrap_or_default();

                if let Some(since) = since_date {
                        if commit_time < since { break; }
                }

                total_scanned += 1;

                let author = commit.author();
                let author_email = author.email().unwrap_or("");
                let is_match = if args.partial {
                    author_email.contains(&args.email)
                } else {
                    author_email == args.email
                };

                if is_match {
                    commits_authored += 1;
                    if args.verbose {
                        print_commit(&commit, &commit_time);
                    }
                }

                if let Some(msg) = commit.message() {
                        if let Some(trailers) = analyze_trailers(msg, &search_email)
                        {
                            if trailers.signed_off && !is_match {
                                signed_off_count += 1;
                            }
                            if trailers.reviewed {
                                reviewed_count += 1;
                            }
                            if trailers.acked {
                                acked_count += 1;
                            }
                            if trailers.tested {
                                tested_count += 1;
                            }
                            if trailers.reported {
                                reported_count += 1;
                            }
                            if !is_match {
                                commits_touched += 1;
                            }
                        } else if !is_match {
                            commits_ignored += 1;
                        }
                } else {
                    // we couldn't read the message
                    dbg!("failed to find msg");
                    commits_ignored += 1;
                }

            debug_assert_eq!(total_scanned, commits_authored + commits_touched + commits_ignored);
        }


        println!("\nSummary:");
        println!("Total Scanned: {}", total_scanned);
        println!("Authored:      {}", commits_authored);
        println!("Touched:       {}", commits_touched);
        println!("Ignored:       {}", commits_ignored);
        println!("\nDetails:");
        println!("Signed-off-by: {}", signed_off_count);
        println!("Reviewed:      {}", reviewed_count);
        println!("Acked:         {}", acked_count);
        println!("Tested:        {}", tested_count);
        println!("Reported:      {}", reported_count);

        println!("Generating Pie Charts...");

        if total_scanned > 0 {
                let data = vec![
                        ("Authored", commits_authored),
                        ("Touched", commits_touched),
                        ("Non Linaro", commits_ignored),
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

fn analyze_trailers(msg: &str, target: &str) -> Option<TrailerState> {
    let mut state = TrailerState::default();

    for line in msg.lines() {
        let lower = line.trim().to_lowercase();
        if lower.contains(target) {
            if lower.starts_with("signed-off-by:") {
                state.signed_off = true;
            } else  if lower.starts_with("reviewed-by:") {
                state.reviewed = true;
            } else if lower.starts_with("acked-by:") {
                state.acked = true;
            } else if lower.starts_with("tested-by:") {
                state.tested = true;
            } else if lower.starts_with("reported-by:") {
                state.reported = true;
            }
        }
    }

    if state.any_active() {
        Some(state)
    } else {
        None
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
