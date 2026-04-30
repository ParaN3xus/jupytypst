use typst::syntax::parse_code;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputStatus {
    Complete,
    Incomplete(String),
    Invalid(String),
}

pub fn classify_input(source: &str) -> InputStatus {
    let errors = parse_code(source).errors();
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
            "expected expression",
            "expected block",
            "expected argument list",
            "expected identifier",
            "expected pattern",
            "expected colon",
        ]
        .iter()
        .any(|prefix| message.starts_with(prefix))
}
