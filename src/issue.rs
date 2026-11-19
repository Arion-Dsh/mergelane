//! MCP input and tool schemas shared by standalone issue writes.

use anyhow::{Context as _, ensure};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    mcp::{object, tool},
    validation::simple_text,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Create {
    pub title: String,
    pub description: Option<String>,
    project: Option<String>,
    confirmed: bool,
}

impl Create {
    pub fn parse(args: Value) -> anyhow::Result<Self> {
        let input: Self = serde_json::from_value(args)?;
        ensure!(
            input.confirmed,
            "the user must confirm before creating an issue"
        );
        validate_title(&input.title)?;
        if let Some(description) = &input.description {
            validate_description(description)?;
        }
        Ok(input)
    }

    pub fn project<'a>(&'a self, default: Option<&'a str>) -> anyhow::Result<&'a str> {
        let project = self
            .project
            .as_deref()
            .or(default)
            .context("provide a project or configure a default project for this platform")?;
        ensure!(
            simple_text(project, 250),
            "provide a single-line project of 1 to 250 bytes"
        );
        Ok(project)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Assign {
    pub issue: String,
    pub assignee: String,
    confirmed: bool,
}

impl Assign {
    pub fn parse(args: Value) -> anyhow::Result<Self> {
        let input: Self = serde_json::from_value(args)?;
        ensure!(
            input.confirmed,
            "the user must confirm before assigning an issue"
        );
        ensure!(
            simple_text(&input.issue, 4096),
            "provide a valid issue link or reference"
        );
        ensure!(
            simple_text(&input.assignee, 255),
            "provide a valid assignee ID"
        );
        Ok(input)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Edit {
    pub issue: String,
    pub title: Option<String>,
    pub description: Option<String>,
    confirmed: bool,
}

impl Edit {
    pub fn parse(args: Value) -> anyhow::Result<Self> {
        let input: Self = serde_json::from_value(args)?;
        ensure!(
            input.confirmed,
            "the user must confirm before editing an issue"
        );
        ensure!(
            simple_text(&input.issue, 4096),
            "provide a valid issue link or reference"
        );
        ensure!(
            input.title.is_some() || input.description.is_some(),
            "provide a title or description to edit"
        );
        if let Some(title) = &input.title {
            validate_title(title)?;
        }
        if let Some(description) = &input.description {
            validate_description(description)?;
        }
        Ok(input)
    }

    pub fn preserve_plan_marker(&mut self, current: &str) {
        let Some(description) = &mut self.description else {
            return;
        };
        let Some(marker) = current.lines().next().filter(|line| {
            line.starts_with("mergelane:task:")
                || (line.starts_with("<!-- mergelane:task:") && line.ends_with(" -->"))
        }) else {
            return;
        };
        if description.is_empty() {
            description.push_str(marker);
        } else {
            description.insert(0, '\n');
            description.insert_str(0, marker);
        }
    }

    pub fn fields(
        &self,
        title_key: &str,
        description_key: &str,
        encode: impl FnOnce(&str) -> Value,
    ) -> Value {
        let mut fields = json!({});
        if let Some(title) = &self.title {
            fields[title_key] = json!(title);
        }
        if let Some(description) = &self.description {
            fields[description_key] = encode(description);
        }
        fields
    }
}

fn validate_title(title: &str) -> anyhow::Result<()> {
    ensure!(
        simple_text(title, 200),
        "provide a single-line issue title of 1 to 200 bytes"
    );
    Ok(())
}

fn validate_description(body: &str) -> anyhow::Result<()> {
    ensure!(
        body.len() <= 8000 && !body.contains("mergelane:") && !body.contains('\0'),
        "issue description must be at most 8000 bytes and contain no reserved markers or NUL"
    );
    Ok(())
}

pub(crate) fn tools(backend: &str, project: &str, member_id: &str) -> [Value; 3] {
    let confirmed = json!({
        "type":"boolean","description":"Set to true after the user explicitly approves the write."
    });
    let mut edit = tool(
        &format!("{backend}_edit_issue"),
        "Edit the title and/or replace the description of an existing issue after user approval. Supply at least one field. Omitted fields are preserved. An empty description clears user-visible text. MergeLane retains any plan recovery marker.",
        object(
            json!({
                "issue":{"type":"string","description":"Issue link or reference on the configured instance."},
                "title":{"type":"string","minLength":1,"maxLength":200},
                "description":{"type":"string","maxLength":8000},
                "confirmed":confirmed
            }),
            &["issue", "confirmed"],
        ),
        false,
    );
    edit["annotations"]["destructiveHint"] = json!(true);
    [
        tool(
            &format!("{backend}_create_issue"),
            "Create one unassigned issue and return its link. Use the configured default project when project is omitted. Obtain user approval before writing. If the result is uncertain, check the project before retrying to avoid duplicate issues.",
            object(
                json!({
                    "title":{"type":"string","minLength":1,"maxLength":200},
                    "description":{"type":"string","maxLength":8000},
                    "project":{"type":"string","description":project},
                    "confirmed":confirmed
                }),
                &["title", "confirmed"],
            ),
            false,
        ),
        tool(
            &format!("{backend}_assign_issue"),
            "Assign an existing unassigned issue to one member after user approval. Use the issue link to hand off across accounts or sessions. An existing assignment to the same member is accepted.",
            object(
                json!({
                    "issue":{"type":"string","description":"Issue link on the configured instance."},
                    "assignee":{"type":"string","description":member_id},
                    "confirmed":confirmed
                }),
                &["issue", "assignee", "confirmed"],
            ),
            false,
        ),
        edit,
    ]
}
