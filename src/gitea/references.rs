use anyhow::{Context as _, ensure};
use percent_encoding::percent_decode_str;
use reqwest::Url;

#[derive(Clone, PartialEq, Eq)]
pub struct IssueRef {
    pub owner: String,
    pub repo: String,
    pub number: u64,
}

impl IssueRef {
    pub fn parse(base: &Url, input: &str) -> anyhow::Result<Self> {
        let (repo, number) = if input.contains("://") {
            instance_path(base, input)?
                .trim_end_matches('/')
                .rsplit_once("/issues/")
                .context("expected a Gitea issue URL")?
        } else {
            input
                .split_once('#')
                .context("expected owner/repo#number or a Gitea issue URL")?
        };
        let (owner, repo) = repository(repo)?;
        let number = number.parse().context("invalid Gitea issue number")?;
        ensure!(number > 0, "issue number must be positive");
        Ok(Self {
            owner: owner.to_ascii_lowercase(),
            repo: repo.to_ascii_lowercase(),
            number,
        })
    }
}

pub fn repository(value: &str) -> anyhow::Result<(&str, &str)> {
    let (owner, repo) = value.split_once('/').context("expected owner/repo")?;
    for component in [owner, repo] {
        ensure!(
            !matches!(component, "" | "." | "..")
                && component.len() <= 255
                && component
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c)),
            "invalid Gitea repository name"
        );
    }
    Ok((owner, repo))
}

fn instance_path<'a>(base: &Url, input: &'a str) -> anyhow::Result<&'a str> {
    let url = Url::parse(input).context("invalid Gitea URL")?;
    ensure!(
        url.origin() == base.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none(),
        "URL must refer to the configured Gitea instance without credentials or query"
    );
    // Preserve the original path to reject literal and encoded dot segments.
    input
        .split('#')
        .next()
        .unwrap_or(input)
        .split_once("://")
        .and_then(|(_, authority)| authority.split_once('/'))
        .and_then(|(_, path)| path.strip_prefix(base.path().trim_start_matches('/')))
        .context("URL is outside the configured Gitea path")
}

#[derive(PartialEq, Eq)]
pub struct Dependency {
    pub owner: String,
    pub repo: String,
    pub commit: String,
    pub path: Vec<String>,
}

pub fn dependencies(base: &Url, body: &str) -> anyhow::Result<Vec<Dependency>> {
    let mut found = Vec::new();
    for url in body.split(|c: char| c.is_whitespace() || "<>()[]`\"'".contains(c)) {
        if !url.starts_with("https://") && !url.starts_with("http://") {
            continue;
        }
        let Ok(relative) = instance_path(base, url) else {
            continue;
        };
        let Some((repo, tail)) = relative.split_once("/src/commit/") else {
            continue;
        };
        let dependency = Dependency::parse(repo, tail)?;
        if !found.contains(&dependency) {
            ensure!(
                found.len() < 8,
                "track at most eight Gitea file links per issue"
            );
            found.push(dependency);
        }
    }
    Ok(found)
}

impl Dependency {
    fn parse(repo: &str, tail: &str) -> anyhow::Result<Self> {
        let (owner, repo) = repository(repo)?;
        let (commit, path) = tail
            .split_once('/')
            .context("file link needs a repository path")?;
        ensure!(
            commit_id(commit),
            "dependency links must use a full Git commit ID"
        );
        let path: Vec<_> = path
            .split('/')
            .map(decode_component)
            .collect::<Result<_, _>>()?;
        ensure!(
            path.iter().map(String::len).sum::<usize>() + path.len() <= 1024,
            "file path is too long"
        );
        Ok(Self {
            owner: owner.to_ascii_lowercase(),
            repo: repo.to_ascii_lowercase(),
            commit: commit.to_ascii_lowercase(),
            path,
        })
    }
}

fn decode_component(segment: &str) -> anyhow::Result<String> {
    let decoded = percent_decode_str(segment).decode_utf8()?;
    ensure!(
        !matches!(decoded.as_ref(), "" | "." | "..")
            && !decoded.contains(['/', '\\'])
            && !decoded.chars().any(char::is_control),
        "invalid file path in dependency link"
    );
    Ok(decoded.into_owned())
}

pub fn commit_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn validate_commit_link(base: &Url, input: &str) -> anyhow::Result<()> {
    let path = instance_path(base, input)?;
    let (repo, commit) = path
        .rsplit_once("/commit/")
        .context("expected a Gitea commit URL")?;
    repository(repo)?;
    ensure!(
        commit_id(commit),
        "checkpoint requires a full Git commit ID"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_pinned_files_on_the_configured_instance() {
        let base = Url::parse("https://git.example.com/gitea/").unwrap();
        let rev = "a".repeat(40);
        let root = format!("{base}team/specs/src/commit/{rev}");
        let link = format!("{root}/docs/hello%20world.md");
        let body = format!(
            "{link}\n[another label]({link}#L2)\nhttps://elsewhere.example/gitea/team/specs/src/commit/{rev}/file.md"
        );
        let files = dependencies(&base, &body).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, ["docs", "hello world.md"]);
        assert!(dependencies(&base, &format!("{root}/docs/%2e%2e/file.md")).is_err());
        assert!(
            IssueRef::parse(&base, "https://git.example.com#gitea/team/specs/issues/7").is_err()
        );
    }
}
