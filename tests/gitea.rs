use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::net::TcpListener;

mod support;

use support::{Mcp, token, tool_result as data};

const OLD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOKEN: &str = "fake-gitea-token";

// Independent MCP processes share the mock instance through request-scoped locking.
struct Fixture {
    base: String,
    issues: Vec<Value>,
    blockers: HashMap<u64, Vec<u64>>,
    comments: HashMap<u64, Vec<Value>>,
    changed: bool,
    fail_link_once: bool,
}

type Data = Arc<Mutex<Fixture>>;
type Params = Query<HashMap<String, String>>;
type Reply = Result<Json<Value>, StatusCode>;

async fn list(State(data): State<Data>, headers: HeaderMap, Query(query): Params) -> Reply {
    authorize(&headers)?;
    let data = data.lock().unwrap();
    if query.get("page").is_some_and(|page| page != "1") {
        return Ok(Json(json!([])));
    }
    Ok(Json(json!(
        data.issues
            .iter()
            .filter(|issue| Some(issue["title"].as_str().unwrap())
                == query.get("q").map(String::as_str))
            .collect::<Vec<_>>()
    )))
}

async fn create(State(data): State<Data>, headers: HeaderMap, Json(body): Json<Value>) -> Reply {
    authorize(&headers)?;
    let mut data = data.lock().unwrap();
    let number = data.issues.len() as u64 + 43;
    let issue = json!({"id":number,"number":number,"state":"open","title":body["title"],"body":body["body"],
        "html_url":format!("{}/team/web/issues/{number}", data.base),
        "assignees":[{"login":body["assignees"][0]}]});
    data.issues.push(issue.clone());
    Ok(Json(issue))
}

async fn search(State(data): State<Data>, headers: HeaderMap, Query(query): Params) -> Reply {
    authorize(&headers)?;
    assert_eq!(query["type"], "issues");
    assert_eq!(query["state"], "all");
    if query["page"] != "1" {
        return Ok(Json(json!([])));
    }
    let data = data.lock().unwrap();
    let issues: Vec<_> = data
        .issues
        .iter()
        .filter(|issue| {
            query
                .get("q")
                .is_none_or(|query| issue["title"].as_str().unwrap().contains(query))
                && (query.get("assigned").map(String::as_str) != Some("true")
                    || issue["assignees"][0]["login"] == "alice")
        })
        .collect();
    Ok(Json(json!(issues)))
}

async fn issue(State(data): State<Data>, Path(number): Path<u64>, headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    let data = data.lock().unwrap();
    if number == 42 {
        return Ok(Json(
            json!({"id":42,"number":42,"state":"open","title":"Export",
            "body":format!("Export spec: [schema]({}/team/specs/src/commit/{OLD}/docs/export%20schema.yaml)", data.base)}),
        ));
    }
    data.issues
        .iter()
        .find(|issue| issue["number"] == number)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_blockers(
    State(data): State<Data>,
    Path(number): Path<u64>,
    Query(query): Params,
    headers: HeaderMap,
) -> Reply {
    authorize(&headers)?;
    let data = data.lock().unwrap();
    let mut blockers = Vec::new();
    if query.get("page").is_none_or(|page| page == "1") {
        for number in data.blockers.get(&number).into_iter().flatten() {
            blockers.push(
                data.issues
                    .iter()
                    .find(|issue| issue["number"] == *number)
                    .unwrap(),
            );
        }
    }
    Ok(Json(json!(blockers)))
}

async fn link(
    State(data): State<Data>,
    Path(number): Path<u64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Reply {
    authorize(&headers)?;
    let mut data = data.lock().unwrap();
    if std::mem::take(&mut data.fail_link_once) {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    data.blockers
        .entry(number)
        .or_default()
        .push(body["index"].as_u64().unwrap());
    Ok(Json(json!({})))
}

async fn comments(State(data): State<Data>, Path(number): Path<u64>, headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    let data = data.lock().unwrap();
    Ok(Json(json!(
        data.comments
            .get(&number)
            .map(Vec::as_slice)
            .unwrap_or_default()
    )))
}

async fn add_comment(
    State(data): State<Data>,
    Path(number): Path<u64>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Reply {
    authorize(&headers)?;
    let mut data = data.lock().unwrap();
    let index = data
        .comments
        .get(&number)
        .map_or(1, |comments| comments.len() + 1);
    let comment = json!({"body":body["body"],"user":{"login":"lead"},
        "html_url":format!("{}/team/web/issues/{number}#issuecomment-{index}", data.base)});
    data.comments
        .entry(number)
        .or_default()
        .push(comment.clone());
    Ok(Json(comment))
}

async fn contents(
    State(data): State<Data>,
    Path(path): Path<String>,
    Query(query): Params,
    headers: HeaderMap,
) -> Reply {
    authorize(&headers)?;
    if path != "docs/export schema.yaml" {
        return Err(StatusCode::NOT_FOUND);
    }
    let is_pinned = query.get("ref").map(String::as_str) == Some(OLD);
    let changed = data.lock().unwrap().changed;
    let blob = if changed && !is_pinned { NEW } else { OLD };
    Ok(Json(
        json!({"type":"file","sha":blob,"last_commit_sha":if is_pinned { OLD } else { NEW }}),
    ))
}

async fn assignable(headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    Ok(Json(json!([{"login":"alice"},{"login":"bob"}])))
}

async fn identity(headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    Ok(Json(json!({"login":"alice"})))
}

fn authorize(headers: &HeaderMap) -> Result<(), StatusCode> {
    (headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some("token fake-gitea-token"))
    .then_some(())
    .ok_or(StatusCode::UNAUTHORIZED)
}

fn start_mcp(base: &str, token: &std::path::Path, write: bool) -> Mcp {
    let mut command = support::command();
    command
        .args(["mcp", "--gitea-url", base, "--gitea-token-file"])
        .arg(token);
    if write {
        command.arg("--gitea-write");
    }
    Mcp::start(command)
}

#[tokio::test]
async fn gitea_only_mcp_plans_assigns_and_shares_versioned_context() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/git", listener.local_addr().unwrap());
    let fixture = Arc::new(Mutex::new(Fixture {
        base: base.clone(),
        issues: vec![],
        blockers: HashMap::new(),
        comments: HashMap::new(),
        changed: false,
        fail_link_once: true,
    }));
    let router = Router::new()
        .route("/git/api/v1/user", get(identity))
        .route("/git/api/v1/repos/issues/search", get(search))
        .route("/git/api/v1/repos/team/web/issues", get(list).post(create))
        .route("/git/api/v1/repos/team/web/issues/{number}", get(issue))
        .route(
            "/git/api/v1/repos/team/web/issues/{number}/dependencies",
            get(list_blockers).post(link),
        )
        .route(
            "/git/api/v1/repos/team/web/issues/{number}/comments",
            get(comments).post(add_comment),
        )
        .route(
            "/git/api/v1/repos/team/specs/contents/{*path}",
            get(contents),
        )
        .route("/git/api/v1/repos/team/web/assignees", get(assignable))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = TempDir::new().unwrap();
    let token = token(dir.path(), "token", TOKEN);

    let doctor = support::command()
        .args(["doctor", "--gitea-url", &base, "--gitea-token-file"])
        .arg(&token)
        .output()
        .await
        .unwrap();
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&doctor.stdout).unwrap()["checks"][0]["backend"],
        "gitea"
    );

    let mut lead = start_mcp(&base, &token, true);
    lead.request("initialize", json!({"protocolVersion":"2025-06-18"}))
        .await;
    let first = data(
        lead.tool("gitea_issue_context", json!({"issue":"team/web#42"}))
            .await,
    );
    assert_eq!(first["dependencies"][0]["status"], "current");
    let file = format!("{base}/team/specs/src/commit/{OLD}/docs/export%20schema.yaml");
    let draft = json!({"parent":"team/web#42","tasks":[
        {"title":"Implement","assignee":"alice","acceptance":["Tests pass"]},
        {"title":"Verify","assignee":"bob","description":file,"acceptance":["Export matches spec"],"after":[0]}
    ]});
    let preview = data(lead.tool("gitea_preview_plan", draft.clone()).await);
    let apply = |confirmed| json!({"plan_id":preview["id"],"confirmed":confirmed});
    assert_eq!(
        lead.tool("gitea_apply_plan", apply(false)).await["isError"],
        true
    );
    assert!(fixture.lock().unwrap().issues.is_empty());
    assert_eq!(
        lead.tool("gitea_apply_plan", apply(true)).await["isError"],
        true
    );
    assert_eq!(fixture.lock().unwrap().issues.len(), 2);

    // Re-preview the same plan after reconnecting. Recover the created issues and finish linking.
    lead.child.kill().await.unwrap();
    let mut lead = start_mcp(&base, &token, true);
    lead.request("initialize", json!({"protocolVersion":"2025-06-18"}))
        .await;
    assert_eq!(
        data(lead.tool("gitea_preview_plan", draft).await)["id"],
        preview["id"]
    );
    let completed = data(lead.tool("gitea_apply_plan", apply(true)).await);
    let second = completed["issues"][1].as_str().unwrap();
    let overview = data(
        lead.tool("gitea_search_issues", json!({"owner":"team"}))
            .await,
    );
    assert_eq!(overview["items"].as_array().unwrap().len(), 2);
    assert_eq!(overview["next_page"], 2);
    let teammate = data(
        lead.tool("gitea_search_issues", json!({"assignee":"bob"}))
            .await,
    );
    assert_eq!(teammate["items"].as_array().unwrap().len(), 1);
    assert_eq!(teammate["items"][0]["url"], second);
    let next = data(lead.tool("gitea_search_issues", json!({"page":2})).await);
    assert!(next["items"].as_array().unwrap().is_empty());
    assert!(next["next_page"].is_null());
    data(lead.tool("gitea_apply_plan", apply(true)).await);
    assert_eq!(fixture.lock().unwrap().issues.len(), 2);
    assert_eq!(fixture.lock().unwrap().comments[&42].len(), 1);
    let parent = data(
        lead.tool("gitea_issue_context", json!({"issue":"team/web#42"}))
            .await,
    );
    assert!(
        parent["activity"]["published_plans"][0]["body"]
            .as_str()
            .unwrap()
            .contains(second)
    );
    data(
        lead.tool(
            "gitea_checkpoint",
            json!({"issue":second,"summary":"Reran the fixture",
        "checks":["Integration passed"],"next":["Send for review"],"confirmed":true}),
        )
        .await,
    );
    fixture.lock().unwrap().changed = true;

    let mut bob = start_mcp(&base, &token, false);
    bob.request("initialize", json!({"protocolVersion":"2025-11-25"}))
        .await;
    let tools = bob.request("tools/list", json!({})).await;
    assert!(
        !tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "gitea_apply_plan")
    );
    let context = data(
        bob.tool("gitea_issue_context", json!({"issue":second}))
            .await,
    );
    assert_eq!(context["issue"]["assignees"][0]["login"], "bob");
    assert_eq!(context["blocking_issues"][0]["title"], "Implement");
    assert_eq!(context["dependencies"][0]["status"], "needs_review");
    assert!(
        context["activity"]["checkpoints"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Reran the fixture")
    );
    assert_eq!(
        bob.tool(
            "gitea_issue_context",
            json!({"issue":"https://other.example.invalid/team/web/issues/42"})
        )
        .await["isError"],
        true
    );
    server.abort();
}
