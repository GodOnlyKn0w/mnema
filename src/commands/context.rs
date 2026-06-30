/// Context-command family: cmd_context plus CLI rendering.
use crate::journal::*;
use crate::output;
use crate::projection;
use crate::util::shorten;

pub(crate) fn cmd_context(
    context_type: Option<&str>,
    covers: &[String],
    since_offset: Option<usize>,
    format_json: Option<&str>,
    exclude_friction: bool,
    include_hidden: bool,
    include_observations: bool,
) -> Result<(), String> {
    let path = ensure_journal()?;
    let (events, _skipped) = read_events_lossy(&path);
    let strands = projection::project_strands(&events, include_hidden);

    let target_type = context_type.unwrap_or("prompt-strand");
    let is_json = format_json == Some("json");

    let view = projection::build_context_view(
        &strands,
        target_type,
        covers,
        since_offset,
        exclude_friction,
        include_observations,
    );

    for warning in &view.warnings {
        eprintln!(
            "{}: [fixed] fixes={} in strand {} does not match any [friction] entry \
             (append_id offset {}) (tasktree explain {})",
            warning.code,
            &warning.fixes_prefix[..warning.fixes_prefix.len().min(12)],
            shorten(&warning.strand_id),
            warning.entry_offset,
            warning.code,
        );
    }

    if is_json {
        let output = output::ContextOutput::from(&view);
        println!(
            "{}",
            serde_json::to_string_pretty(&output).map_err(|e| format!("serialize error: {}", e))?
        );
    } else {
        print_context_text(&view);
    }

    Ok(())
}

fn print_context_text(view: &projection::ContextView) {
    println!("# Strand Context\n");
    let strand_count = view.strands.len();
    for (i, strand) in view.strands.iter().enumerate() {
        let covers_str = if strand.covers.is_empty() {
            String::new()
        } else {
            format!(" [covers: {}]", strand.covers.join(", "))
        };
        println!(
            "## prompt-strand:{} <id:{}>",
            covers_str,
            shorten(&strand.id)
        );
        for entry in &strand.entries {
            if entry.marker.is_empty() {
                println!("  {}", entry.content);
            } else {
                println!("  {} {}", entry.marker, entry.content);
            }
        }
        if strand.friction_folded > 0 {
            println!(
                "  friction: ×{} (folded — strand closed; tasktree show {})",
                strand.friction_folded,
                shorten(&strand.id)
            );
        }
        if strand.folded_counts.any_folded() {
            let fc = &strand.folded_counts;
            println!(
                "  folded: progress ×{} | observed ×{} | check ×{}  (tasktree show {})",
                fc.progress,
                fc.observed,
                fc.check,
                shorten(&strand.id)
            );
        }
        if i + 1 < strand_count {
            println!();
        }
    }
}
