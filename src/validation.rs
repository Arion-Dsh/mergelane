//! Pure validation shared by the Gitea and Plane adapters.

/// Accept nonempty, trimmed, control-free text up to a UTF-8 byte limit.
pub(crate) fn simple_text(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
