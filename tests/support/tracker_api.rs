//! Provider-shaped mock APIs. Writes can persist before an error response is returned.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{Method, StatusCode},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use tokio::{net::TcpListener, task::JoinHandle};

#[derive(Clone, Copy)]
pub enum Platform {
    GitHub,
    GitLab,
    Jira,
}

impl Platform {
    pub fn name(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
            Self::Jira => "jira",
        }
    }
    pub fn project(self) -> &'static str {
        if matches!(self, Self::Jira) {
            "WEB"
        } else {
            "team/web"
        }
    }
    pub fn assignee(self, second: bool) -> &'static str {
        match (self, second) {
            (Self::GitHub, false) => "alice",
            (Self::GitHub, true) => "bob",
            (Self::GitLab, false) => "1",
            (Self::GitLab, true) => "2",
            (Self::Jira, false) => "alice-id",
            (Self::Jira, true) => "bob-id",
        }
    }
    pub fn key(self, number: u64) -> String {
        if matches!(self, Self::Jira) {
            format!("WEB-{number}")
        } else {
            format!("team/web#{number}")
        }
    }
}

struct Task {
    number: u64,
    title: String,
    body: String,
    assignee: Option<String>,
    label: String,
}

pub struct Fixture {
    platform: Platform,
    base: String,
    tasks: Vec<Task>,
    comments: HashMap<u64, Vec<String>>,
    blockers: HashMap<u64, Vec<u64>>,
    fail_create: bool,
    fail_link: bool,
    fail_summary: bool,
    pub requests: usize,
}

impl Fixture {
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.tasks.len(),
            self.blockers.values().map(Vec::len).sum(),
            self.comments.values().map(Vec::len).sum(),
        )
    }

    fn task(&self, number: u64) -> &Task {
        self.tasks
            .iter()
            .find(|task| task.number == number)
            .expect("known mock task")
    }

    fn issue(&self, number: u64) -> Value {
        let parent = Task {
            number: 42,
            title: "Billing export".to_owned(),
            body: "Shared requirement".to_owned(),
            assignee: Some(self.platform.assignee(false).to_owned()),
            label: String::new(),
        };
        let task = if number == 42 {
            &parent
        } else {
            self.task(number)
        };
        let base = &self.base;
        match self.platform {
            Platform::GitHub => {
                json!({"id":1000+number,"number":number,"title":task.title,"body":task.body,"state":"open",
                "html_url":format!("{base}/team/web/issues/{number}"),
                "assignees":task.assignee.iter().map(|id| json!({"login":id})).collect::<Vec<_>>()})
            }
            Platform::GitLab => {
                json!({"id":1000+number,"iid":number,"project_id":7,"title":task.title,"description":task.body,"state":"opened",
                "web_url":format!("{base}/team/web/-/issues/{number}"),
                "assignees":task.assignee.iter().map(|id| json!({"id":id.parse::<u64>().unwrap(),"username":if id == "1" {"alice"} else {"bob"}})).collect::<Vec<_>>()})
            }
            Platform::Jira => {
                json!({"id":(1000+number).to_string(),"key":format!("WEB-{number}"),"fields":{
                "summary":task.title,"description":adf(&task.body),"status":{"name":"In Progress"},"project":{"key":"WEB"},
                "assignee":task.assignee.as_ref().map(|id| json!({"accountId":id,"displayName":id})),"labels":[task.label],
                "issuelinks":self.blockers.get(&number).into_iter().flatten().map(|blocker| json!({"type":{"name":"Blocks","inward":"is blocked by","outward":"blocks"},"inwardIssue":{"key":format!("WEB-{blocker}")}})).collect::<Vec<_>>()}})
            }
        }
    }

    fn create(&mut self, body: &Value) -> Result<Value, StatusCode> {
        let number = 43 + self.tasks.len() as u64;
        let task = match self.platform {
            Platform::GitHub => Task {
                number,
                title: string(body, "title"),
                body: body["body"].as_str().unwrap_or_default().to_owned(),
                assignee: body["assignees"][0].as_str().map(str::to_owned),
                label: String::new(),
            },
            Platform::GitLab => Task {
                number,
                title: string(body, "title"),
                body: body["description"].as_str().unwrap_or_default().to_owned(),
                assignee: body["assignee_ids"][0].as_u64().map(|id| id.to_string()),
                label: String::new(),
            },
            Platform::Jira => {
                let fields = &body["fields"];
                assert_eq!(fields["project"]["key"], "WEB");
                assert_eq!(fields["issuetype"]["name"], "Task");
                Task {
                    number,
                    title: string(fields, "summary"),
                    body: if fields["description"].is_null() {
                        String::new()
                    } else {
                        plain(&fields["description"])
                    },
                    assignee: fields["assignee"]["accountId"].as_str().map(str::to_owned),
                    label: fields["labels"][0].as_str().unwrap_or_default().to_owned(),
                }
            }
        };
        self.tasks.push(task);
        if self.fail_create && number == 44 {
            self.fail_create = false;
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        if matches!(self.platform, Platform::Jira) {
            Ok(json!({"id":(1000+number).to_string(),"key":format!("WEB-{number}")}))
        } else {
            Ok(self.issue(number))
        }
    }

    fn assign(&mut self, number: u64, assignee: String) -> Result<Value, StatusCode> {
        if ![self.platform.assignee(false), self.platform.assignee(true)]
            .contains(&assignee.as_str())
        {
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let task = self
            .tasks
            .iter_mut()
            .find(|task| task.number == number)
            .ok_or(StatusCode::NOT_FOUND)?;
        assert!(
            task.assignee.is_none(),
            "the test client must not overwrite an assignment"
        );
        task.assignee = Some(assignee);
        if matches!(self.platform, Platform::Jira) {
            return Ok(Value::Null);
        }
        Ok(self.issue(number))
    }

    fn edit(&mut self, number: u64, body: &Value) -> Result<Value, StatusCode> {
        let task = self
            .tasks
            .iter_mut()
            .find(|task| task.number == number)
            .ok_or(StatusCode::NOT_FOUND)?;
        let (fields, title, description) = match self.platform {
            Platform::GitHub => (body, "title", "body"),
            Platform::GitLab => (body, "title", "description"),
            Platform::Jira => (&body["fields"], "summary", "description"),
        };
        assert!(
            fields
                .as_object()
                .unwrap()
                .keys()
                .all(|key| key == title || key == description)
        );
        if let Some(value) = fields.get(title) {
            task.title = value.as_str().unwrap().to_owned();
        }
        if let Some(value) = fields.get(description) {
            task.body = match self.platform {
                Platform::Jira if value.is_null() => String::new(),
                Platform::Jira => plain(value),
                _ => value.as_str().unwrap().to_owned(),
            };
        }
        if matches!(self.platform, Platform::Jira) {
            return Ok(Value::Null);
        }
        Ok(self.issue(number))
    }

    fn link(&mut self, number: u64, blocker: u64) -> Result<Value, StatusCode> {
        assert_eq!(
            (number, blocker),
            (44, 43),
            "blocker direction must match the approved plan"
        );
        if self.fail_link {
            self.fail_link = false;
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        let links = self.blockers.entry(number).or_default();
        assert!(!links.contains(&blocker), "duplicate link write");
        links.push(blocker);
        Ok(Value::Null)
    }

    fn comment(&mut self, number: u64, body: String) -> Result<Value, StatusCode> {
        let summary = body.starts_with("mergelane:plan:");
        self.comments.entry(number).or_default().push(body);
        if summary && self.fail_summary {
            self.fail_summary = false;
            return Err(StatusCode::SERVICE_UNAVAILABLE);
        }
        Ok(json!({"id":self.comments[&number].len()}))
    }
}

pub struct Server {
    pub base: String,
    pub fixture: Arc<Mutex<Fixture>>,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Server {
    pub async fn start(platform: Platform) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!(
            "http://{}/{}",
            listener.local_addr().unwrap(),
            platform.name()
        );
        let fixture = Arc::new(Mutex::new(Fixture {
            platform,
            base: base.clone(),
            tasks: Vec::new(),
            comments: HashMap::new(),
            blockers: HashMap::new(),
            fail_create: true,
            fail_link: true,
            fail_summary: true,
            requests: 0,
        }));
        let app = Router::new().fallback(api).with_state(fixture.clone());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            base,
            fixture,
            task,
        }
    }
}

async fn api(State(fixture): State<Arc<Mutex<Fixture>>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 1024 * 1024).await.unwrap();
    let body: Value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap()
    };
    let url = reqwest::Url::parse(&format!("http://fixture{}", parts.uri)).unwrap();
    let path: Vec<_> = url
        .path_segments()
        .unwrap()
        .map(|part| {
            percent_encoding::percent_decode_str(part)
                .decode_utf8()
                .unwrap()
                .into_owned()
        })
        .collect();
    let query: HashMap<_, _> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let mut fixture = fixture.lock().unwrap();
    fixture.requests += 1;
    let authorized = match fixture.platform {
        Platform::GitHub => {
            parts.headers.get("authorization").is_some_and(|value| {
                value == "Bearer fake-token" || value == "Bearer manager-token"
            }) && parts.headers.get("user-agent").is_some()
                && parts
                    .headers
                    .get("x-github-api-version")
                    .is_some_and(|value| value == "2026-03-10")
        }
        Platform::GitLab => {
            parts
                .headers
                .get("private-token")
                .is_some_and(|value| value == "fake-token" || value == "manager-token")
                && !parts.headers.contains_key("authorization")
        }
        Platform::Jira => parts.headers.get("authorization").is_some_and(|value| {
            value == "Basic dGVzdEBleGFtcGxlLmNvbTpmYWtlLXRva2Vu"
                || value == "Basic dGVzdEBleGFtcGxlLmNvbTptYW5hZ2VyLXRva2Vu"
        }),
    };
    if !authorized {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let result = match fixture.platform {
        Platform::GitHub => github(&mut fixture, &path[3..], &parts.method, &query, &body),
        Platform::GitLab => gitlab(&mut fixture, &path[3..], &parts.method, &query, &body),
        Platform::Jira => jira(&mut fixture, &path[4..], &parts.method, &query, &body),
    };
    match result {
        Ok(Value::Null) if parts.method == Method::PUT => StatusCode::NO_CONTENT.into_response(),
        Ok(Value::Null) => (StatusCode::CREATED, "").into_response(),
        Ok(value) => Json(value).into_response(),
        Err(status) => status.into_response(),
    }
}

fn github(
    f: &mut Fixture,
    path: &[String],
    method: &Method,
    query: &HashMap<String, String>,
    body: &Value,
) -> Result<Value, StatusCode> {
    let p: Vec<_> = path.iter().map(String::as_str).collect();
    match p.as_slice() {
        ["user"] => Ok(json!({"login":"alice"})),
        ["search", "issues"] if query["q"].contains("is:open") => {
            assert_eq!(*method, Method::GET);
            assert!(query["q"].contains("assignee:alice") || query["q"].contains("repo:team/web"));
            Ok(json!({"total_count":31,"incomplete_results":false,"items":[f.issue(42)]}))
        }
        ["search", "issues"] => {
            assert!(query["q"].contains("repo:team/web"));
            let items: Vec<_> = if query.get("page").is_some_and(|page| page != "1") {
                vec![]
            } else {
                f.tasks.iter().map(|task| f.issue(task.number)).collect()
            };
            Ok(json!({"total_count":f.tasks.len(),"incomplete_results":false,"items":items}))
        }
        ["repos", "team", "web", "assignees"] => Ok(json!([{"login":"alice"},{"login":"bob"}])),
        ["repos", "team", "web", "issues"] if *method == Method::POST => f.create(body),
        ["repos", "team", "web", "issues"] => Ok(json!(
            f.tasks
                .iter()
                .map(|task| f.issue(task.number))
                .collect::<Vec<_>>()
        )),
        ["repos", "team", "web", "issues", number] if *method == Method::PATCH => {
            f.edit(number.parse().unwrap(), body)
        }
        ["repos", "team", "web", "issues", number] => Ok(f.issue(number.parse().unwrap())),
        ["repos", "team", "web", "issues", number, "assignees"] if *method == Method::POST => f
            .assign(
                number.parse().unwrap(),
                body["assignees"][0].as_str().unwrap().to_owned(),
            ),
        ["repos", "team", "web", "issues", number, "comments"] if *method == Method::POST => {
            f.comment(number.parse().unwrap(), string(body, "body"))
        }
        ["repos", "team", "web", "issues", number, "comments"] => Ok(json!(
            f.comments
                .get(&number.parse().unwrap())
                .into_iter()
                .flatten()
                .map(|body| json!({"body":body,"html_url":format!("{}/comment",f.base)}))
                .collect::<Vec<_>>()
        )),
        [
            "repos",
            "team",
            "web",
            "issues",
            number,
            "dependencies",
            "blocked_by",
        ] if *method == Method::POST => f.link(
            number.parse().unwrap(),
            body["issue_id"].as_u64().unwrap() - 1000,
        ),
        [
            "repos",
            "team",
            "web",
            "issues",
            number,
            "dependencies",
            "blocked_by",
        ] => Ok(json!(
            f.blockers
                .get(&number.parse().unwrap())
                .into_iter()
                .flatten()
                .map(|number| f.issue(*number))
                .collect::<Vec<_>>()
        )),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

fn gitlab(
    f: &mut Fixture,
    path: &[String],
    method: &Method,
    query: &HashMap<String, String>,
    body: &Value,
) -> Result<Value, StatusCode> {
    let p: Vec<_> = path.iter().map(String::as_str).collect();
    match p.as_slice() {
        ["user"] => Ok(json!({"id":1,"username":"alice"})),
        ["projects", "team/web", "members", "all"] => {
            Ok(json!([{"id":1,"username":"alice"},{"id":2,"username":"bob"}]))
        }
        ["projects", "team/web", "issues"] if *method == Method::POST => f.create(body),
        ["issues"] if query["state"] == "opened" => {
            assert_eq!(*method, Method::GET);
            assert_eq!(query["scope"], "assigned_to_me");
            Ok(json!([f.issue(42)]))
        }
        ["projects", "team/web", "issues"] => Ok(json!(if query
            .get("page")
            .is_some_and(|page| page != "1")
        {
            vec![]
        } else {
            f.tasks
                .iter()
                .map(|task| f.issue(task.number))
                .collect::<Vec<_>>()
        })),
        ["projects", "team/web", "issues", number] if *method == Method::PUT => {
            let number = number.parse().unwrap();
            if body.get("assignee_ids").is_some() {
                f.assign(
                    number,
                    body["assignee_ids"][0].as_u64().unwrap().to_string(),
                )
            } else {
                f.edit(number, body)
            }
        }
        ["projects", "team/web", "issues", number] => Ok(f.issue(number.parse().unwrap())),
        ["projects", "team/web", "issues", number, "notes"] if *method == Method::POST => {
            f.comment(number.parse().unwrap(), string(body, "body"))
        }
        ["projects", "team/web", "issues", number, "notes"] => Ok(json!(
            f.comments
                .get(&number.parse().unwrap())
                .into_iter()
                .flatten()
                .map(|body| json!({"body":body,"system":false}))
                .collect::<Vec<_>>()
        )),
        ["projects", "team/web", "issues", number, "links"] if *method == Method::POST => {
            assert_eq!(body["target_project_id"], "team/web");
            assert_eq!(body["link_type"], "is_blocked_by");
            f.link(
                number.parse().unwrap(),
                body["target_issue_iid"].as_u64().unwrap(),
            )
        }
        ["projects", "team/web", "issues", number, "links"] => Ok(json!(
            f.blockers
                .get(&number.parse().unwrap())
                .into_iter()
                .flatten()
                .map(|number| {
                    let mut issue = f.issue(*number);
                    issue["link_type"] = json!("is_blocked_by");
                    issue
                })
                .collect::<Vec<_>>()
        )),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

fn jira(
    f: &mut Fixture,
    path: &[String],
    method: &Method,
    query: &HashMap<String, String>,
    body: &Value,
) -> Result<Value, StatusCode> {
    let p: Vec<_> = path.iter().map(String::as_str).collect();
    match p.as_slice() {
        ["myself"] => Ok(json!({"accountId":"alice-id"})),
        ["user", "assignable", "search"] => {
            assert_eq!(query["project"], "WEB");
            Ok(
                json!([{"accountId":"alice-id","displayName":"Alice","active":true},{"accountId":"bob-id","displayName":"Bob","active":true}]),
            )
        }
        ["search", "jql"] => {
            let jql = &query["jql"];
            if jql.contains("statusCategory != Done") {
                assert_eq!(*method, Method::GET);
                assert!(jql.contains("assignee = currentUser()"));
                return if query.contains_key("nextPageToken") {
                    Ok(json!({"issues":[],"isLast":true}))
                } else {
                    Ok(json!({"issues":[f.issue(42)],"isLast":false,"nextPageToken":"pull-page-2"}))
                };
            }
            let label = jql
                .split("labels = ")
                .nth(1)
                .map(|value| serde_json::from_str::<String>(value).unwrap());
            let items = f
                .tasks
                .iter()
                .filter(|task| label.as_ref().is_none_or(|label| &task.label == label))
                .map(|task| f.issue(task.number))
                .collect::<Vec<_>>();
            if label.is_none() && !query.contains_key("nextPageToken") {
                Ok(json!({"issues":items,"isLast":false,"nextPageToken":"page-2"}))
            } else if query.contains_key("nextPageToken") {
                Ok(json!({"issues":[],"isLast":true}))
            } else {
                Ok(json!({"issues":items,"isLast":true}))
            }
        }
        ["issue"] if *method == Method::POST => f.create(body),
        ["issue", key] if *method == Method::PUT => f.edit(number(key), body),
        ["issue", key] => Ok(f.issue(number(key))),
        ["issue", key, "assignee"] if *method == Method::PUT => {
            f.assign(number(key), string(body, "accountId"))
        }
        ["issue", key, "comment"] if *method == Method::POST => {
            f.comment(number(key), plain(&body["body"]))
        }
        ["issue", key, "comment"] => {
            let comments = f
                .comments
                .get(&number(key))
                .into_iter()
                .flatten()
                .map(|body| json!({"body":adf(body)}))
                .collect::<Vec<_>>();
            Ok(json!({"total":comments.len(),"comments":comments}))
        }
        ["issueLink"] if *method == Method::POST => {
            assert_eq!(body["type"]["name"], "Blocks");
            f.link(
                number(body["outwardIssue"]["key"].as_str().unwrap()),
                number(body["inwardIssue"]["key"].as_str().unwrap()),
            )
        }
        _ => Err(StatusCode::NOT_FOUND),
    }
}

fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap().to_owned()
}
fn number(key: &str) -> u64 {
    key.strip_prefix("WEB-").unwrap().parse().unwrap()
}
fn adf(text: &str) -> Value {
    json!({"type":"doc","version":1,"content":text.split('\n').map(|line| json!({"type":"paragraph","content":[{"type":"text","text":line}]})).collect::<Vec<_>>()})
}
fn plain(value: &Value) -> String {
    assert_eq!(value["type"], "doc");
    assert_eq!(value["version"], 1);
    value["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|paragraph| {
            paragraph["content"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| part["text"].as_str().unwrap())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
