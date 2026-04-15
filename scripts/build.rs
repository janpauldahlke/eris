use std::path::PathBuf;

fn should_generate() -> bool {
    if std::env::var("GWS_GEN").as_deref() == Ok("1") {
        return true;
    }
    let dir = PathBuf::from("src/generated/gws_types");
    if !dir.exists() {
        return true;
    }
    let has_rs = std::fs::read_dir(&dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .any(|e| e.path().extension().is_some_and(|ext| ext == "rs"))
        })
        .unwrap_or(false);
    !has_rs
}

fn main() {
    if !should_generate() {
        println!("cargo:warning=gws-builder: skipping generation (src/generated/gws_types/ already populated; set GWS_GEN=1 to force)");
        return;
    }

    let result = gws_builder::generate(gws_builder::BuilderConfig {
        services: match build_services() {
            Ok(s) => s,
            Err(e) => {
                println!("cargo:warning=gws-builder: service spec error: {e}");
                return;
            }
        },
        out_dir: PathBuf::from("src/generated/gws_types"),
        regeneration: gws_builder::RegenerationPolicy::IfChanged,
        fetcher: None,
        cache_dir: None,
    });

    match result {
        Ok(report) => {
            eprintln!(
                "gws-builder: generated {} actions, {} schemas; skipped: {:?}",
                report.actions_emitted, report.schemas_emitted, report.services_skipped
            );
        }
        Err(e) => {
            println!("cargo:warning=gws-builder: generation failed: {e} — build will proceed without generated types");
        }
    }
}

fn build_services() -> Result<Vec<gws_builder::ServiceSpec>, Box<dyn std::error::Error>> {
    Ok(vec![gws_builder::ServiceSpec::whitelist(
        "gmail",
        "v1",
        vec![
            "users.messages.list".into(),
            "users.messages.get".into(),
            "users.messages.send".into(),
        ],
    )?])
}
