//! ADF conversion preserves paragraph boundaries used by the recovery markers.

use anyhow::ensure;
use serde_json::{Value, json};

pub(super) fn adf(text: &str) -> Value {
    json!({"type":"doc","version":1,"content":text.split('\n').map(|line| {
        let content = if line.is_empty() { Vec::new() } else { vec![json!({"type":"text","text":line})] };
        json!({"type":"paragraph","content":content})
    }).collect::<Vec<_>>()})
}

pub(super) fn plain(value: &Value) -> anyhow::Result<String> {
    let mut output = String::new();
    render(value, 0, &mut output)?;
    Ok(output.trim_end_matches('\n').to_owned())
}

fn render(value: &Value, depth: usize, output: &mut String) -> anyhow::Result<()> {
    ensure!(depth <= 32, "Jira document nesting exceeds 32 levels");
    if value.is_null() {
        return Ok(());
    }
    if let Some(text) = value.as_str().or_else(|| value["text"].as_str()) {
        output.push_str(text);
    }
    if value["type"] == "hardBreak" {
        output.push('\n');
    }
    if let Some(content) = value["content"].as_array() {
        for child in content {
            render(child, depth + 1, output)?;
        }
    }
    if matches!(value["type"].as_str(), Some("paragraph" | "heading")) {
        output.push('\n');
    }
    ensure!(
        output.len() <= 1024 * 1024,
        "Jira document text exceeds 1 MiB"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_markers_and_untrusted_text_survive_adf_roundtrip() {
        let text = "mergelane:task:abc:0\n<script>&\"\n\nNext action";
        assert_eq!(plain(&adf(text)).unwrap(), text);
        let mut nested = json!({"type":"text","text":"deep"});
        for _ in 0..34 {
            nested = json!({"type":"paragraph","content":[nested]});
        }
        assert!(plain(&nested).is_err());
    }
}
