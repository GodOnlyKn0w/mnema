use crate::cli::Cli;
use clap::CommandFactory;

fn splitish(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in line.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push(std::mem::take(&mut cur));
                    }
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

fn substitute(tok: &str) -> String {
    if !tok.contains('<') {
        return tok.to_string();
    }
    let upper = tok.to_uppercase();
    if upper.contains("ID") {
        "0000019dd34b".to_string()
    } else if upper.contains("<N>") {
        "5".to_string()
    } else if upper.contains("FORMAT") {
        "json".to_string()
    } else if upper.contains("PATH") || upper.contains("FILE") {
        "x.md".to_string()
    } else if upper.contains("CODE") {
        "W062".to_string()
    } else if upper.contains("RFC3339") {
        "2026-01-01T00:00:00Z".to_string()
    } else {
        "x".to_string()
    }
}

pub(in crate::tests) fn try_parse_example(line: &str) -> Result<(), String> {
    let start = match line.find("tasktree ") {
        Some(i) => i,
        None => return Ok(()),
    };
    if line.contains("[--") {
        return Ok(());
    }
    let cmdline = line[start..].trim_end_matches(['.', ',', ';', ':', ')']);
    let tokens: Vec<String> = splitish(cmdline).iter().map(|t| substitute(t)).collect();
    match Cli::command().try_get_matches_from(&tokens) {
        Ok(_) => Ok(()),
        Err(e) => match e.kind() {
            clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => Ok(()),
            _ => Err(format!(
                "example does not parse: `{}` -> {}",
                cmdline.trim(),
                e
            )),
        },
    }
}
