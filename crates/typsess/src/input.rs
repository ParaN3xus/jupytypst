use typst::syntax::{Source, parse_code};

use crate::SourceMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputStatus {
    Complete,
    Incomplete(String),
    Invalid(String),
}

pub fn classify_input(source: &str, mode: SourceMode) -> InputStatus {
    let errors = match mode {
        SourceMode::Code => parse_code(source).errors(),
        SourceMode::Markup => Source::detached(source).root().errors(),
    };
    if errors.is_empty() {
        return InputStatus::Complete;
    }

    let message = errors
        .into_iter()
        .map(|error| error.message.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    if is_incomplete_input(&message) {
        InputStatus::Incomplete(message)
    } else {
        InputStatus::Invalid(message)
    }
}

fn is_incomplete_input(message: &str) -> bool {
    let message = message.trim();
    message.starts_with("unclosed ")
        || [
            "expected block",
            "expected argument list",
            "expected identifier",
            "expected pattern",
            "expected colon",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_complete_input() {
        assert_eq!(
            classify_input("let x = 1", SourceMode::Code),
            InputStatus::Complete
        );
    }

    #[test]
    fn classifies_incomplete_input() {
        for (source, expected_message) in [
            ("{", "unclosed delimiter"),
            ("{ x", "unclosed delimiter"),
            ("\"", "unclosed string"),
            ("1; \"", "unclosed string"),
            ("if true", "expected block"),
            ("let x = if true", "expected block"),
            ("set text", "expected argument list"),
            ("1; set text", "expected argument list"),
            ("set", "expected identifier"),
            ("1; set", "expected identifier"),
            ("let", "expected pattern"),
            ("1; let", "expected pattern"),
            ("show heading", "expected colon"),
            ("1; show heading", "expected colon"),
        ] {
            assert_incomplete(source, expected_message);
        }
    }

    #[test]
    fn classifies_invalid_input() {
        assert!(matches!(
            classify_input("let x = 1 2", SourceMode::Code),
            InputStatus::Invalid(_)
        ));
    }

    fn assert_incomplete(source: &str, expected_message: &str) {
        match classify_input(source, SourceMode::Code) {
            InputStatus::Incomplete(message) => {
                assert_eq!(message, expected_message, "source: {source:?}");
            }
            status => panic!("expected incomplete input for {source:?}, got {status:?}"),
        }
    }
}
