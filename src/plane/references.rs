use anyhow::{Context as _, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkItemRef {
    pub workspace: String,
    pub project: Uuid,
    pub item: Uuid,
}

pub(super) enum WorkItemUrl {
    Id(WorkItemRef),
    Identifier {
        workspace: String,
        identifier: String,
    },
}

impl WorkItemUrl {
    pub fn parse(base: &Url, reference: &str) -> anyhow::Result<Self> {
        let url = Url::parse(reference).context("provide a full Plane work item URL")?;
        ensure!(
            url.origin() == base.origin()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none(),
            "work item URL must belong to the configured Plane instance"
        );
        let path = url
            .path()
            .strip_prefix(base.path())
            .context("work item is outside the configured Plane path")?;
        let parts: Vec<_> = path.trim_end_matches('/').split('/').collect();
        ensure!(
            parts.first().is_some_and(|slug| valid_slug(slug)),
            "invalid Plane workspace slug"
        );
        if let [workspace, "browse", identifier] = parts.as_slice() {
            let (project, number) = identifier
                .rsplit_once('-')
                .context("expected a Plane identifier such as PROJ-123")?;
            ensure!(
                valid_slug(project) && number.parse::<u64>().is_ok_and(|number| number > 0),
                "invalid Plane work item identifier"
            );
            return Ok(Self::Identifier {
                workspace: (*workspace).to_owned(),
                identifier: (*identifier).to_owned(),
            });
        }
        ensure!(
            parts.len() == 5
                && parts[1] == "projects"
                && matches!(parts[3], "issues" | "work-items"),
            "expected /workspace/browse/PROJ-123/ or a project work item UUID URL"
        );
        Ok(Self::Id(WorkItemRef {
            workspace: parts[0].to_owned(),
            project: Uuid::parse_str(parts[2]).context("invalid Plane project UUID")?,
            item: Uuid::parse_str(parts[4]).context("invalid Plane work item UUID")?,
        }))
    }
}

pub(super) fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 255
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
}

/// Render user-provided text as HTML text, preserving line breaks.
pub(super) fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('\n', "<br>")
}

/// Extract visible text and hrefs from Plane's rich-text descriptions for file tracking.
pub(super) fn file_links(content: &str) -> String {
    content
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_instance_path_and_renders_untrusted_text() {
        let base = Url::parse("https://plane.example.com/plane/").unwrap();
        let project = Uuid::nil();
        let url = format!("{base}team/projects/{project}/issues/{project}/");
        assert!(
            matches!(WorkItemUrl::parse(&base, &url).unwrap(), WorkItemUrl::Id(item) if item.workspace == "team")
        );
        assert!(WorkItemUrl::parse(&base, &url.replace("/plane/", "/other/")).is_err());
        assert!(
            WorkItemUrl::parse(
                &base,
                &url.replace("plane.example.com", "other.example.com")
            )
            .is_err()
        );
        assert!(
            matches!(WorkItemUrl::parse(&base, &format!("{base}team/browse/PROJ-123/")).unwrap(),
            WorkItemUrl::Identifier { workspace, identifier } if workspace == "team" && identifier == "PROJ-123")
        );
        for path in [
            "team/browse/PROJ-0/",
            "team/browse/PROJ-no/",
            "team/browse/PROJ-1/extra",
            "team/browse/PROJ-1/?token=x",
        ] {
            assert!(WorkItemUrl::parse(&base, &format!("{base}{path}")).is_err());
        }
        assert_eq!(html("<script>\n&\""), "&lt;script&gt;<br>&amp;&quot;");
    }
}
