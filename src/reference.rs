use crate::projection::ProjectedStrand;
use crate::util::{shorten, truncate};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct SelectionState {
    journal_path: String,
    max_offset: usize,
    last_touched: Option<String>,
    last_list: Vec<SelectionMapping>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SelectionMapping {
    index: usize,
    id: String,
    last_offset: usize,
}

fn selection_state_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::journal::resolve_journal_dir()?.join("selection-state.json"))
}

fn current_journal_path_string() -> Result<String, String> {
    Ok(crate::journal::ensure_journal()?.display().to_string())
}

fn read_state() -> Result<SelectionState, String> {
    let path = selection_state_path()?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("selection cache unavailable; run tasktree list: {}", e))?;
    serde_json::from_str(&text)
        .map_err(|e| format!("selection cache is corrupt; rerun tasktree list: {}", e))
}

fn read_state_for_update() -> SelectionState {
    read_state().unwrap_or_default()
}

fn write_state(state: &SelectionState) -> Result<(), String> {
    let path = selection_state_path()?;
    let tmp = path.with_extension("selection-state.tmp");
    let text = serde_json::to_string_pretty(state)
        .map_err(|e| format!("cannot serialize selection cache: {}", e))?;
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write selection cache: {}", e))?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("cannot replace selection cache: {}", e))?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("cannot replace selection cache: {}", e))?;
    Ok(())
}

pub(crate) fn remember_last_touched_current(strand_id: &str) -> Result<(), String> {
    let path = crate::journal::ensure_journal()?;
    let mut state = read_state_for_update();
    state.journal_path = path.display().to_string();
    state.last_touched = Some(strand_id.to_string());
    write_state(&state)
}

pub(crate) fn remember_list(strands: &[ProjectedStrand], max_offset: usize) -> Result<(), String> {
    let mut state = read_state_for_update();
    state.journal_path = current_journal_path_string()?;
    state.max_offset = max_offset;
    state.last_list = strands
        .iter()
        .enumerate()
        .map(|(idx, strand)| SelectionMapping {
            index: idx + 1,
            id: strand.id.clone(),
            last_offset: strand.last_offset(),
        })
        .collect();
    write_state(&state)
}

fn resolve_selection_handle(
    strands: &[ProjectedStrand],
    input: &str,
    current_max_offset: usize,
) -> StrandLookup {
    let state = match read_state() {
        Ok(state) => state,
        Err(e) => return StrandLookup::Invalid(e),
    };
    if input == "@last" {
        let Some(id) = state.last_touched else {
            return StrandLookup::Invalid(
                "@last is not set; run tasktree show/add/append first".to_string(),
            );
        };
        if strands.iter().any(|s| s.id == id) {
            return StrandLookup::One(id);
        }
        return StrandLookup::Invalid(
            "@last points to a missing strand; use an explicit id".to_string(),
        );
    }
    let Some(index_text) = input.strip_prefix('@') else {
        return StrandLookup::Invalid(format!("unknown selection handle {}", input));
    };
    let Ok(index) = index_text.parse::<usize>() else {
        return StrandLookup::Invalid(format!("unknown selection handle {}", input));
    };
    if state.max_offset != current_max_offset {
        return StrandLookup::Invalid(format!(
            "{} is stale: journal advanced from offset {} to {}; rerun tasktree list",
            input, state.max_offset, current_max_offset
        ));
    }
    let Some(mapping) = state
        .last_list
        .iter()
        .find(|mapping| mapping.index == index)
    else {
        return StrandLookup::Invalid(format!(
            "{} is not in the last list output; rerun tasktree list",
            input
        ));
    };
    let Some(strand) = strands.iter().find(|strand| strand.id == mapping.id) else {
        return StrandLookup::Invalid(format!(
            "{} points to a missing strand; rerun tasktree list",
            input
        ));
    };
    if strand.last_offset() != mapping.last_offset {
        return StrandLookup::Invalid(format!(
            "{} is stale: strand {} moved from offset {} to {}; rerun tasktree list",
            input,
            shorten(&strand.id),
            mapping.last_offset,
            strand.last_offset()
        ));
    }
    StrandLookup::One(mapping.id.clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StrandCandidate {
    pub(crate) id: String,
    pub(crate) slug: Option<String>,
    pub(crate) summary: String,
    pub(crate) state: String,
    pub(crate) last_offset: usize,
}

impl StrandCandidate {
    fn from_strand(strand: &ProjectedStrand) -> Self {
        Self {
            id: strand.id.clone(),
            slug: strand.slug.clone(),
            summary: truncate(strand.first_summary(), 50),
            state: strand.state().to_string(),
            last_offset: strand.last_offset(),
        }
    }

    fn label(&self) -> String {
        let slug = self
            .slug
            .as_ref()
            .map(|s| format!(" slug={}", s))
            .unwrap_or_default();
        format!(
            "{}{} {} offset={} \"{}\"",
            shorten(&self.id),
            slug,
            self.state,
            self.last_offset,
            self.summary
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StrandLookup {
    One(String),
    NotFound,
    Ambiguous(Vec<StrandCandidate>),
    Invalid(String),
}

impl StrandLookup {
    pub(crate) fn into_result(self, input: &str) -> Result<String, String> {
        match self {
            StrandLookup::One(id) => Ok(id),
            StrandLookup::NotFound => Err(format!("strand {} not found", input)),
            StrandLookup::Invalid(message) => Err(message),
            StrandLookup::Ambiguous(candidates) => Err(ambiguous_message(input, &candidates)),
        }
    }
}

pub(crate) fn ambiguous_message(input: &str, candidates: &[StrandCandidate]) -> String {
    let sample: Vec<String> = candidates
        .iter()
        .take(4)
        .map(StrandCandidate::label)
        .collect();
    format!(
        "strand handle {} is ambiguous: {} strands match (e.g. {})",
        input,
        candidates.len(),
        sample.join("; ")
    )
}

/// Resolve a human strand handle against the canonical full strand set.
/// Command-specific filtering happens after this step so one handle cannot
/// point at different strands in different commands.
pub(crate) fn lookup_strand(strands: &[ProjectedStrand], input: &str) -> StrandLookup {
    lookup_strand_with_selection(strands, input, false, 0)
}

pub(crate) fn lookup_strand_with_selection(
    strands: &[ProjectedStrand],
    input: &str,
    allow_selection: bool,
    current_max_offset: usize,
) -> StrandLookup {
    let input = input.trim();
    if input.is_empty() {
        return StrandLookup::Invalid("strand handle cannot be empty".to_string());
    }
    if input.starts_with('@') {
        if allow_selection {
            return resolve_selection_handle(strands, input, current_max_offset);
        }
        return StrandLookup::Invalid(format!(
            "selection handle {} is text-mode only and unavailable here",
            input
        ));
    }

    let mut candidates: Vec<StrandCandidate> = Vec::new();
    for strand in strands {
        let slug_match = strand.slug.as_deref() == Some(input);
        let id_match = strand.id.starts_with(input);
        if slug_match || id_match {
            if !candidates.iter().any(|candidate| candidate.id == strand.id) {
                candidates.push(StrandCandidate::from_strand(strand));
            }
        }
    }

    match candidates.len() {
        0 => StrandLookup::NotFound,
        1 => StrandLookup::One(candidates[0].id.clone()),
        _ => StrandLookup::Ambiguous(candidates),
    }
}

pub(crate) fn resolve_strand(strands: &[ProjectedStrand], input: &str) -> Result<String, String> {
    lookup_strand(strands, input).into_result(input)
}

pub(crate) fn resolve_strand_with_selection(
    strands: &[ProjectedStrand],
    input: &str,
    allow_selection: bool,
    current_max_offset: usize,
) -> Result<String, String> {
    lookup_strand_with_selection(strands, input, allow_selection, current_max_offset)
        .into_result(input)
}

pub(crate) fn validate_slug(slug: &str) -> Result<(), String> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err("--slug cannot be empty".to_string());
    }
    if slug == "@last" || (slug.starts_with('@') && slug[1..].chars().all(|c| c.is_ascii_digit())) {
        return Err("--slug cannot use reserved @ selection syntax".to_string());
    }
    let mut chars = slug.chars();
    let first = chars.next().expect("slug is non-empty");
    if !first.is_ascii_alphanumeric() {
        return Err("--slug must start with an ASCII letter or digit".to_string());
    }
    if !std::iter::once(first)
        .chain(chars)
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return Err("--slug may contain only ASCII letters, digits, '.', '_' and '-'".to_string());
    }
    if slug.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "--slug must not be pure hex; slug and hash prefixes are separate namespaces"
                .to_string(),
        );
    }
    Ok(())
}
