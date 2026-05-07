use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args();
    let bin = args
        .next()
        .unwrap_or_else(|| "moltbook_soak_check".to_string());
    let Some(log_path) = args.next() else {
        eprintln!("usage: {bin} <logfile>");
        return ExitCode::from(2);
    };
    if args.next().is_some() {
        eprintln!("usage: {bin} <logfile>");
        return ExitCode::from(2);
    }

    let path = Path::new(&log_path);
    if !path.exists() || !path.is_file() {
        eprintln!("log file not found: {}", path.display());
        eprintln!("usage: {bin} <logfile>");
        return ExitCode::from(2);
    }

    let body = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("failed reading {}: {e}", path.display());
            return ExitCode::from(1);
        }
    };
    let lines: Vec<&str> = body.lines().collect();

    println!("=== repeat_failure_suppressed events (count) ===");
    println!(
        "{}",
        count_any(&lines, &["orchestrator.tools.repeat_failure_suppressed"])
    );

    println!("\n=== moltbook.browse.batch_ledger (sample up to 5) ===");
    print_samples(&lines, &["moltbook.browse.batch_ledger"], 5);

    println!("\n=== agenda.remind_at.xor_normalized (count) ===");
    println!("{}", count_any(&lines, &["agenda.remind_at.xor_normalized"]));

    println!("\n=== Moltbook response JSON parse (non-schema recovery path) ===");
    println!(
        "{}",
        count_any(
            &lines,
            &[
                "Moltbook JSON parsed after stripping illegal control characters",
                "moltbook.response.json_sanitized",
            ],
        )
    );

    println!("\n=== Targeted schema recovery lines mentioning moltbook (manual review) ===");
    print_samples_any_group(
        &lines,
        &[
            &["Retrying with targeted schemas", "moltbook"],
            &["targeted_tools", "moltbook"],
        ],
        20,
    );

    println!("\nDone. Soak pass: review counts above; confirm engagement floor met or last_blocker present in ledger lines.");
    ExitCode::SUCCESS
}

fn count_any(lines: &[&str], needles: &[&str]) -> usize {
    lines
        .iter()
        .filter(|line| needles.iter().any(|needle| line.contains(needle)))
        .count()
}

fn print_samples(lines: &[&str], needles: &[&str], max: usize) {
    let mut printed = 0usize;
    for line in lines {
        if matches_all(line, needles) {
            println!("{line}");
            printed = printed.saturating_add(1);
            if printed >= max {
                break;
            }
        }
    }
    if printed == 0 {
        println!("(none)");
    }
}

fn matches_all(line: &str, needles: &[&str]) -> bool {
    needles.iter().all(|needle| line.contains(needle))
}

fn print_samples_any_group(lines: &[&str], groups: &[&[&str]], max: usize) {
    let mut printed = 0usize;
    for line in lines {
        if groups.iter().any(|group| matches_all(line, group)) {
            println!("{line}");
            printed = printed.saturating_add(1);
            if printed >= max {
                break;
            }
        }
    }
    if printed == 0 {
        println!("(none)");
    }
}
