//! Jira Cloud REST v3 adapter with ADF text and enhanced JQL search.

mod text;

use anyhow::{Context as _, ensure};
use reqwest::Url;
use serde_json::{Value, json};

use crate::tracker::{Api, Comment, Http, Issue, Member, Search, Task, append, instance_path};
use text::{adf, plain};

pub(crate) struct Jira {
    http: Http,
    issue_type: String,
    link_type: String,
}

pub(crate) struct Options {
    pub email: String,
    pub issue_type: String,
    pub link_type: String,
    pub cloud_id: Option<String>,
}

impl Jira {
    pub fn new(base: Url, token: String, options: Options) -> anyhow::Result<Self> {
        let Options {
            email,
            issue_type,
            link_type,
            cloud_id,
        } = options;
        ensure!(
            crate::validation::simple_text(&issue_type, 100)
                && crate::validation::simple_text(&link_type, 100),
            "invalid Jira issue or link type"
        );
        let mut http = Http::basic("Jira", base, email, token)?;
        http.api = append(http.web.clone(), &["rest", "api", "3"]);
        if let Some(cloud_id) = cloud_id {
            let cloud_id = uuid::Uuid::parse_str(&cloud_id)
                .context("invalid Jira Cloud ID")?
                .to_string();
            http.api = append(
                Url::parse("https://api.atlassian.com/").expect("static API URL"),
                &["ex", "jira", &cloud_id, "rest", "api", "3"],
            );
        }
        Ok(Self {
            http,
            issue_type,
            link_type,
        })
    }

    fn key(&self, reference: &str) -> anyhow::Result<String> {
        let key = if reference.contains("://") {
            instance_path(&self.http.web, reference)?
                .strip_prefix("browse/")
                .context("expected a Jira browse URL")?
                .to_owned()
        } else {
            reference.to_owned()
        };
        let (project, number) = key.rsplit_once('-').context("expected a Jira issue key")?;
        self.project(project)?;
        ensure!(
            number.parse::<u64>().is_ok_and(|n| n > 0),
            "invalid Jira issue number"
        );
        Ok(key.to_ascii_uppercase())
    }

    fn decode(&self, value: &Value) -> anyhow::Result<Issue> {
        let key = self.key(required(value, "key")?)?;
        let fields = &value["fields"];
        let project = self.project(required(&fields["project"], "key")?)?;
        ensure!(
            key.rsplit_once('-')
                .is_some_and(|(prefix, _)| prefix == project),
            "Jira issue key does not match its project"
        );
        let assignees = if fields["assignee"].is_null() {
            Vec::new()
        } else {
            vec![member(&fields["assignee"])?]
        };
        Ok(Issue {
            id: required(value, "id")?.to_owned(),
            key: key.clone(),
            project,
            url: append(self.http.web.clone(), &["browse", &key]).to_string(),
            title: required(fields, "summary")?.to_owned(),
            description: plain(&fields["description"])?,
            state: required(&fields["status"], "name")?.to_owned(),
            assignees,
        })
    }

    async fn read(&self, key: &str) -> anyhow::Result<Value> {
        self.http
            .get(
                &["issue", key],
                &[(
                    "fields",
                    "summary,description,status,assignee,project,issuelinks".to_owned(),
                )],
            )
            .await
    }

    fn issue_type(&self) -> Value {
        if self.issue_type.bytes().all(|c| c.is_ascii_digit()) {
            json!({"id":self.issue_type})
        } else {
            json!({"name":self.issue_type})
        }
    }

    async fn search_page(&self, jql: &str, cursor: Option<String>) -> anyhow::Result<Value> {
        let mut query = vec![
            ("jql", jql.to_owned()),
            ("maxResults", "30".to_owned()),
            (
                "fields",
                "summary,description,status,assignee,project".to_owned(),
            ),
        ];
        if let Some(cursor) = cursor {
            query.push(("nextPageToken", cursor));
        }
        self.http.get(&["search", "jql"], &query).await
    }
}

impl Api for Jira {
    fn instance(&self) -> &Url {
        &self.http.web
    }

    async fn check_connection(&self) -> anyhow::Result<()> {
        let value = self.http.get(&["myself"], &[]).await?;
        required(&value, "accountId")?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "jira"
    }

    fn project(&self, value: &str) -> anyhow::Result<String> {
        ensure!(
            !value.is_empty()
                && value.len() <= 100
                && value.as_bytes()[0].is_ascii_alphabetic()
                && value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_'),
            "invalid Jira project key"
        );
        Ok(value.to_ascii_uppercase())
    }

    async fn search(&self, input: Search) -> anyhow::Result<Value> {
        ensure!(
            input.page == 1,
            "Jira uses cursor pagination; provide next_cursor as cursor"
        );
        ensure!(
            !(input.mine && input.assignee.is_some()),
            "choose mine or assignee"
        );
        let mut clauses = Vec::new();
        if let Some(project) = input.project {
            clauses.push(format!("project = {}", quote(&self.project(&project)?)?));
        }
        if input.mine {
            clauses.push("assignee = currentUser()".to_owned());
        }
        if let Some(assignee) = input.assignee {
            clauses.push(format!("assignee = {}", quote(&assignee)?));
        }
        if let Some(query) = input.query {
            clauses.push(if let Ok(key) = self.key(&query) {
                format!("key = {}", quote(&key)?)
            } else {
                format!("summary ~ {}", quote(&query)?)
            });
        }
        ensure!(
            !clauses.is_empty(),
            "provide a Jira project, query, assignee or mine to bound the search"
        );
        if input.open_only {
            clauses.push("statusCategory != Done".to_owned());
        }
        let value = self
            .search_page(
                &format!("{} ORDER BY updated DESC", clauses.join(" AND ")),
                input.cursor,
            )
            .await?;
        let items = value["issues"]
            .as_array()
            .context("Jira search returned no issues array")?
            .iter()
            .map(|value| self.decode(value))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let cursor = value["nextPageToken"].as_str();
        ensure!(
            value["isLast"] != false || cursor.is_some(),
            "Jira omitted its next search cursor"
        );
        Ok(
            json!({"items":items,"next_cursor":cursor,"notice":"Jira search indexing can lag recent writes; inspect remote state before retrying uncertain creates."}),
        )
    }

    async fn issue(&self, reference: &str) -> anyhow::Result<Issue> {
        let key = self.key(reference)?;
        let issue = self.decode(&self.read(&key).await?)?;
        ensure!(issue.key == key, "Jira returned a different issue");
        Ok(issue)
    }

    async fn members(&self, project: &str) -> anyhow::Result<Vec<Member>> {
        let project = self.project(project)?;
        let mut all = Vec::new();
        for page in 0..20 {
            let value = self
                .http
                .get(
                    &["user", "assignable", "search"],
                    &[
                        ("project", project.clone()),
                        ("startAt", (page * 100).to_string()),
                        ("maxResults", "100".to_owned()),
                    ],
                )
                .await?;
            let batch = value
                .as_array()
                .context("Jira assignable members must be an array")?;
            all.extend(
                batch
                    .iter()
                    .filter(|user| user["active"] != false)
                    .map(member)
                    .collect::<anyhow::Result<Vec<_>>>()?,
            );
            if batch.len() < 100 {
                return Ok(all);
            }
        }
        anyhow::bail!("Jira member list exceeded 2000 entries")
    }

    async fn comments(&self, issue: &Issue) -> anyhow::Result<Vec<Comment>> {
        let mut all = Vec::new();
        for page in 0..20 {
            let value = self
                .http
                .get(
                    &["issue", &issue.key, "comment"],
                    &[
                        ("startAt", (page * 100).to_string()),
                        ("maxResults", "100".to_owned()),
                    ],
                )
                .await?;
            let batch = value["comments"]
                .as_array()
                .context("Jira comments must be an array")?;
            for comment in batch {
                all.push(Comment {
                    body: plain(&comment["body"])?,
                    url: None,
                });
            }
            let total = value["total"]
                .as_u64()
                .context("Jira comments have no total")?;
            if all.len() as u64 >= total {
                return Ok(all);
            }
            ensure!(
                !batch.is_empty(),
                "Jira comment pagination made no progress"
            );
        }
        anyhow::bail!("Jira comments exceeded 2000 entries")
    }

    async fn blockers(&self, issue: &Issue) -> anyhow::Result<Vec<String>> {
        let value = self.read(&issue.key).await?;
        let links = value["fields"]["issuelinks"]
            .as_array()
            .context("Jira omitted issue links")?;
        links
            .iter()
            .filter(|link| link["type"]["name"] == self.link_type && !link["inwardIssue"].is_null())
            .map(|link| self.key(required(&link["inwardIssue"], "key")?))
            .collect()
    }

    async fn find_task(&self, project: &str, marker: &str) -> anyhow::Result<Option<Issue>> {
        let label = marker.replace(':', "-");
        let value = self
            .search_page(
                &format!(
                    "project = {} AND labels = {}",
                    quote(&self.project(project)?)?,
                    quote(&label)?
                ),
                None,
            )
            .await?;
        let items = value["issues"]
            .as_array()
            .context("Jira task search returned no issues")?;
        ensure!(
            items.len() <= 1 && value["nextPageToken"].is_null(),
            "duplicate Jira plan tasks; resolve them remotely"
        );
        items.first().map(|value| self.decode(value)).transpose()
    }

    async fn create(&self, task: &Task, body: &str, marker: &str) -> anyhow::Result<Issue> {
        let value = self.http.post(&["issue"], &json!({"fields":{"project":{"key":task.project()?},"summary":task.title,
            "description":adf(body),"issuetype":self.issue_type(),"assignee":{"accountId":task.assignee},"labels":[marker.replace(':', "-")]}})).await?;
        self.issue(required(&value, "key")?).await
    }

    async fn create_unassigned(
        &self,
        project: &str,
        input: &crate::issue::Create,
    ) -> anyhow::Result<Issue> {
        let mut fields = json!({
            "project":{"key":project},"summary":input.title,
            "issuetype":self.issue_type(),"assignee":null
        });
        if let Some(description) = input.description.as_deref().filter(|text| !text.is_empty()) {
            fields["description"] = adf(description);
        }
        let value = self
            .http
            .post(&["issue"], &json!({"fields":fields}))
            .await?;
        let key = self.key(required(&value, "key")?)?;
        self.issue(&key).await.with_context(|| {
            let url = append(self.http.web.clone(), &["browse", &key]);
            format!("Jira created {url} but its assignment could not be read")
        })
    }

    async fn assign(&self, issue: &Issue, assignee: &str) -> anyhow::Result<Issue> {
        ensure!(
            assignee != "-1",
            "provide a Jira accountId for a specific member"
        );
        self.http
            .put(
                &["issue", &issue.key, "assignee"],
                &json!({"accountId":assignee}),
            )
            .await?;
        self.issue(&issue.key).await
    }

    async fn edit(&self, issue: &Issue, input: &crate::issue::Edit) -> anyhow::Result<Issue> {
        let fields = input.fields("summary", "description", |text| {
            if text.is_empty() {
                Value::Null
            } else {
                adf(text)
            }
        });
        self.http
            .put(&["issue", &issue.key], &json!({"fields":fields}))
            .await?;
        self.issue(&issue.key).await
    }

    async fn block(&self, issue: &Issue, blocker: &Issue) -> anyhow::Result<()> {
        // The blocker occupies the inward end; the blocked issue sees inwardIssue in GET.
        self.http
            .post(
                &["issueLink"],
                &json!({"type":{"name":self.link_type},
            "inwardIssue":{"key":blocker.key},"outwardIssue":{"key":issue.key}}),
            )
            .await?;
        Ok(())
    }

    async fn comment(&self, issue: &Issue, body: &str) -> anyhow::Result<()> {
        self.http
            .post(
                &["issue", &issue.key, "comment"],
                &json!({"body":adf(body)}),
            )
            .await?;
        Ok(())
    }
}

fn required<'a>(value: &'a Value, field: &str) -> anyhow::Result<&'a str> {
    value[field]
        .as_str()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Jira response is missing {field}"))
}

fn member(value: &Value) -> anyhow::Result<Member> {
    Ok(Member {
        id: required(value, "accountId")?.to_owned(),
        name: required(value, "displayName")?.to_owned(),
    })
}

fn quote(value: &str) -> anyhow::Result<String> {
    Ok(serde_json::to_string(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_tokens_use_the_official_gateway_while_issue_links_keep_the_site() {
        let jira = Jira::new(
            Url::parse("https://team.atlassian.net/").unwrap(),
            "fake-token".to_owned(),
            Options {
                email: "test@example.com".to_owned(),
                issue_type: "Task".to_owned(),
                link_type: "Blocks".to_owned(),
                cloud_id: Some("11111111-1111-4111-8111-111111111111".to_owned()),
            },
        )
        .unwrap();
        assert_eq!(
            jira.http.api.as_str(),
            "https://api.atlassian.com/ex/jira/11111111-1111-4111-8111-111111111111/rest/api/3"
        );
        assert_eq!(jira.http.web.as_str(), "https://team.atlassian.net/");
        assert_eq!(
            jira.key("https://team.atlassian.net/browse/WEB-42")
                .unwrap(),
            "WEB-42"
        );
        assert!(
            jira.key("https://other.atlassian.net/browse/WEB-42")
                .is_err()
        );
    }
}
