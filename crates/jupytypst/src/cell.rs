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
            mode = parse_directive(rest)?;
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

fn parse_directive(rest: &str) -> Result<RenderMode> {
    let rest = rest.trim();
    let Some(value) = rest.strip_prefix("mode=").map(str::trim) else {
        return Err(anyhow!("unsupported jupytypst directive `{rest}`"));
    };
    match value {
        "svg" => Ok(RenderMode::Svg),
        "html" => Ok(RenderMode::Html),
        other => Err(anyhow!("unsupported jupytypst mode `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_comment_mode_directive() {
        let cell = parse_cell("// jupytypst: mode=svg\n[Test]", RenderMode::Html).unwrap();
        assert_eq!(cell.mode, RenderMode::Svg);
        assert_eq!(cell.body, "[Test]");
    }

    #[test]
    fn rejects_unsupported_mode() {
        let error = parse_cell("// jupytypst: mode=pdf\n1 + 2", RenderMode::Svg).unwrap_err();
        assert!(error.to_string().contains("unsupported jupytypst mode"));
    }

    #[test]
    fn keeps_default_mode_without_directive() {
        let cell = parse_cell("[Test]", RenderMode::Svg).unwrap();
        assert_eq!(cell.mode, RenderMode::Svg);
        assert_eq!(cell.body, "[Test]");
    }
}
