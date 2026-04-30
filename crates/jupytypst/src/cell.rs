use anyhow::{Result, anyhow};
use typsess::RenderMode;

#[derive(Debug, PartialEq, Eq)]
pub struct ParsedCell {
    pub mode: RenderMode,
    pub body: String,
}

pub fn parse_cell(source: &str, default_mode: RenderMode) -> Result<ParsedCell> {
    let mut mode = default_mode;
    let mut body_start = 0;

    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            body_start += line.len();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("// jupytypst:") {
            mode = parse_directive(rest, "format")?;
            body_start += line.len();
            continue;
        }
        break;
    }

    Ok(ParsedCell {
        mode,
        body: source[body_start..].to_string(),
    })
}

fn parse_directive(rest: &str, field: &str) -> Result<RenderMode> {
    let rest = rest.trim();
    let Some(value) = rest.strip_prefix(&format!("{field}=")).map(str::trim) else {
        return Err(anyhow!("unsupported jupytypst directive `{rest}`"));
    };
    match value {
        "svg" => Ok(RenderMode::Svg),
        "html" => Ok(RenderMode::Html),
        other => Err(anyhow!("unsupported jupytypst format `{other}`")),
    }
}
