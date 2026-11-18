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

use support::{Mcp, token, tool_result as result};

const PROJECT: &str = "11111111-1111-4111-8111-111111111111";
const PARENT: &str = "22222222-2222-4222-8222-222222222222";
const ALICE: &str = "33333333-3333-4333-8333-333333333333";
const BOB: &str = "44444444-4444-4444-8444-444444444444";
const OLD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const NEW: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const STARTED: &str = "77777777-7777-4777-8777-777777777777";

// Test requests share fixture state across independent MCP child processes.
#[derive(Default)]
struct Fixture {
    base: String,
    items: Vec<Value>,
    comments: HashMap<String, Vec<Value>>,
    blockers: HashMap<String, Vec<String>>,
    fail_create_once: bool,
    fail_relation_once: bool,
    changed: bool,
    repeat_cursor: bool,
}

type Data = Arc<Mutex<Fixture>>;
type Params = Query<HashMap<String, String>>;
type Reply = Result<Json<Value>, StatusCode>;

fn authorize(headers: &HeaderMap) -> Result<(), StatusCode> {
    (headers
        .get("x-api-key")
        .and_then(|header| header.to_str().ok())
        == Some("fake-plane-key")
        && !headers.contains_key("authorization"))
    .then_some(())
    .ok_or(StatusCode::UNAUTHORIZED)
}

fn page(results: Vec<Value>, next: bool) -> Value {
    json!({"results":results,"next_page_results":next,"next_cursor":"100:1:0"})
}

async fn read(
    State(data): State<Data>,
    Path((project, tail)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Params,
) -> Reply {
    authorize(&headers)?;
    assert_eq!(project, PROJECT);
    let data = data.lock().unwrap();
    let parts: Vec<_> = tail.trim_end_matches('/').split('/').collect();
    match parts.as_slice() {
        ["members"] => Ok(Json(json!([
            {"id":ALICE,"display_name":"Alice"}, {"id":BOB,"display_name":"Bob"}
        ]))),
        ["work-items"] => {
            let Some(external) = query.get("external_id") else {
                return Ok(Json(page(data.items.clone(), false)));
            };
            data.items
                .iter()
                .find(|item| item["external_id"] == *external)
                .cloned()
                .map(Json)
                .ok_or(StatusCode::NOT_FOUND)
        }
        ["states"] => Ok(Json(
            json!([{"id":STARTED,"name":"In progress","group":"started"}]),
        )),
        ["work-items", id] => {
            if *id == PARENT {
                return Ok(Json(
                    json!({"id":PARENT,"project":PROJECT,"name":"Export","sequence_id":1,
                        "description_html":format!("<p><a href=\"{}/gitea/team/specs/src/commit/{OLD}/schema.yaml\">Spec</a></p>",data.base)
                    }),
                ));
            }
            data.items
                .iter()
                .find(|item| item["id"] == *id)
                .cloned()
                .map(Json)
                .ok_or(StatusCode::NOT_FOUND)
        }
        ["work-items", id, "comments"] => {
            let comments = data.comments.get(*id).cloned().unwrap_or_default();
            if data.repeat_cursor {
                return Ok(Json(page(vec![], true)));
            }
            // Force the client to follow cursor pages to see the saved summary.
            if query.contains_key("cursor") {
                Ok(Json(page(comments, false)))
            } else {
                Ok(Json(page(vec![], true)))
            }
        }
        ["work-items", id, "relations"] => Ok(Json(json!({
            "blocking":[],
            "blocked_by":data.blockers.get(*id).into_iter().flatten()
                .map(|id| json!({"project_id":PROJECT,"issue_id":id})).collect::<Vec<_>>()
        }))),
        _ => Err(StatusCode::NOT_FOUND),
    }
}

async fn write(
    State(data): State<Data>,
    Path((project, tail)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Reply {
    authorize(&headers)?;
    assert_eq!(project, PROJECT);
    let mut data = data.lock().unwrap();
    let parts: Vec<_> = tail.trim_end_matches('/').split('/').collect();
    match parts.as_slice() {
        ["work-items"] => {
            let id = format!("55555555-5555-4555-8555-{:012}", data.items.len() + 1);
            let mut item = body;
            item["id"] = json!(id);
            item["project"] = json!(PROJECT);
            item["state"] = json!(STARTED);
            item["sequence_id"] = json!(data.items.len() + 2);
            assert_eq!(item["parent"], PARENT);
            data.items.push(item.clone());
            if std::mem::take(&mut data.fail_create_once) {
                // Model an uncertain response after the server has committed the write.
                return Err(StatusCode::BAD_GATEWAY);
            }
            Ok(Json(item))
        }
        ["work-items", id, "relations"] => {
            if std::mem::take(&mut data.fail_relation_once) {
                return Err(StatusCode::SERVICE_UNAVAILABLE);
            }
            assert_eq!(body["relation_type"], "blocked_by");
            data.blockers.entry(id.to_string()).or_default().extend(
                body["issues"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_str().unwrap().to_owned()),
            );
            Ok(Json(json!({"message":"success"})))
        }
        ["work-items", id, "comments"] => {
            assert_eq!(body["access"], "INTERNAL");
            let mut comment = body;
            comment["id"] = json!(format!(
                "66666666-6666-4666-8666-{:012}",
                data.comments.values().map(Vec::len).sum::<usize>() + 1
            ));
            comment["actor"] = json!(ALICE);
            data.comments
                .entry(id.to_string())
                .or_default()
                .push(comment.clone());
            Ok(Json(comment))
        }
        _ => Err(StatusCode::NOT_FOUND),
    }
}

async fn contents(State(data): State<Data>, Query(query): Params, headers: HeaderMap) -> Reply {
    assert!(!headers.contains_key("x-api-key"));
    assert_eq!(
        headers.get("authorization").unwrap(),
        "token fake-gitea-key"
    );
    let baseline = query.get("ref").map(String::as_str) == Some(OLD);
    let changed = data.lock().unwrap().changed;
    Ok(Json(
        json!({"type":"file", "sha":if !baseline && changed { NEW } else { OLD },
        "last_commit_sha":if baseline { OLD } else { NEW }}),
    ))
}

async fn projects(headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    Ok(Json(page(
        vec![json!({"id":PROJECT,"name":"Billing","identifier":"BILL"})],
        false,
    )))
}

async fn project(headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    Ok(Json(
        json!({"id":PROJECT,"name":"Billing","identifier":"BILL"}),
    ))
}

async fn by_identifier(
    State(data): State<Data>,
    Path(identifier): Path<String>,
    headers: HeaderMap,
) -> Reply {
    authorize(&headers)?;
    let data = data.lock().unwrap();
    if identifier == "BILL-1" {
        return Ok(Json(
            json!({"id":PARENT,"project":PROJECT,"name":"Export","sequence_id":1}),
        ));
    }
    data.items
        .iter()
        .find(|item| format!("BILL-{}", item["sequence_id"]) == identifier)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn identity(headers: HeaderMap) -> Reply {
    authorize(&headers)?;
    Ok(Json(json!({"id":ALICE})))
}

async fn search(State(data): State<Data>, headers: HeaderMap, Query(query): Params) -> Reply {
    authorize(&headers)?;
    assert_eq!(query["workspace_search"], "false");
    assert_eq!(query["project_id"], PROJECT);
    let data = data.lock().unwrap();
    Ok(Json(json!({"issues":data.items.iter().filter(|item| {
        item["name"].as_str().unwrap().to_lowercase().contains(&query["search"].to_lowercase())
    }).map(|item| json!({"id":item["id"],"project_id":PROJECT})).collect::<Vec<_>>()})))
}

fn start_mcp(
    base: &str,
    token: &std::path::Path,
    gitea: Option<&std::path::Path>,
    write: bool,
) -> Mcp {
    let mut cmd = support::command();
    cmd.args([
        "mcp",
        "--plane-url",
        &format!("{base}/plane"),
        "--plane-token-file",
    ])
    .arg(token);
    if let Some(token) = gitea {
        cmd.args([
            "--gitea-url",
            &format!("{base}/gitea"),
            "--gitea-token-file",
        ])
        .arg(token);
    }
    cmd.args(["--plane-workspace", "team"]);
    if write {
        cmd.arg("--plane-write");
    }
    Mcp::start(cmd)
}

#[tokio::test]
async fn plane_ce_mcp_splits_assigns_recovers_and_shares_context_with_git_tracking() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let fixture = Arc::new(Mutex::new(Fixture {
        base: base.clone(),
        fail_create_once: true,
        fail_relation_once: true,
        ..Fixture::default()
    }));
    let router = Router::new()
        .route("/plane/api/v1/workspaces/team/projects/", get(projects))
        .route(
            "/plane/api/v1/workspaces/team/projects/{project}/",
            get(project),
        )
        .route(
            "/plane/api/v1/workspaces/team/work-items/search/",
            get(search),
        )
        .route(
            "/plane/api/v1/workspaces/team/work-items/{identifier}/",
            get(by_identifier),
        )
        .route("/plane/api/v1/users/me/", get(identity))
        .route(
            "/plane/api/v1/workspaces/team/projects/{project}/{*tail}",
            get(read).post(write),
        )
        .route(
            "/gitea/api/v1/repos/team/specs/contents/schema.yaml",
            get(contents),
        )
        .with_state(fixture.clone());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let dir = TempDir::new().unwrap();
    let plane_token = token(dir.path(), "plane-token", "fake-plane-key");
    let gitea_token = token(dir.path(), "gitea-token", "fake-gitea-key");

    let doctor = support::command()
        .args([
            "doctor",
            "--plane-url",
            &format!("{base}/plane"),
            "--plane-token-file",
        ])
        .arg(&plane_token)
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
        "plane"
    );
    let parent = format!("{base}/plane/team/projects/{PROJECT}/issues/{PARENT}/");
    let mut lead = start_mcp(&base, &plane_token, None, true);
    lead.request("initialize", json!({"protocolVersion":"2025-06-18"}))
        .await;
    let tools = lead.request("tools/list", json!({})).await;
    assert_eq!(tools["tools"].as_array().unwrap().len(), 9);
    let context = result(
        lead.tool("plane_issue_context", json!({"issue":parent}))
            .await,
    );
    assert_eq!(context["members"][1]["display_name"], "Bob");
    assert_eq!(context["dependency_tracking"]["status"], "not_configured");
    assert_eq!(context["url"], format!("{base}/plane/team/browse/BILL-1/"));
    let draft = json!({"parent":parent,"tasks":[
        {"title":"Implement","assignee":ALICE,"acceptance":["Exports correctly"]},
        {"title":"Verify","assignee":BOB,"description":format!("{base}/gitea/team/specs/src/commit/{OLD}/schema.yaml"),
            "acceptance":["Matches <schema> & examples"],"after":[0]}
    ]});
    let preview = result(lead.tool("plane_preview_plan", draft.clone()).await);
    let apply = |confirmed| json!({"plan_id":preview["id"],"confirmed":confirmed});
    assert_eq!(
        lead.tool("plane_apply_plan", apply(false)).await["isError"],
        true
    );
    assert!(fixture.lock().unwrap().items.is_empty());
    assert_eq!(
        lead.tool("plane_apply_plan", apply(true)).await["isError"],
        true
    );
    assert_eq!(fixture.lock().unwrap().items.len(), 1);
    lead.child.kill().await.unwrap();
    let mut lead = start_mcp(&base, &plane_token, None, true);
    lead.request("initialize", json!({"protocolVersion":"2025-06-18"}))
        .await;
    assert_eq!(
        result(lead.tool("plane_preview_plan", draft).await)["id"],
        preview["id"]
    );
    assert_eq!(
        lead.tool("plane_apply_plan", apply(true)).await["isError"],
        true
    );
    let published = result(lead.tool("plane_apply_plan", apply(true)).await);
    result(lead.tool("plane_apply_plan", apply(true)).await);
    {
        let fixture = fixture.lock().unwrap();
        assert_eq!(fixture.items.len(), 2);
        assert_eq!(fixture.comments[PARENT].len(), 1);
        assert!(
            fixture.items[1]["description_html"]
                .as_str()
                .unwrap()
                .contains("&lt;schema&gt; &amp; examples")
        );
    }
    let second = published["issues"][1].as_str().unwrap();
    assert_eq!(second, format!("{base}/plane/team/browse/BILL-3/"));
    let preview_from_browser_link = result(
        lead.tool(
            "plane_preview_plan",
            json!({
                "parent":format!("{base}/plane/team/browse/BILL-1/"),
                "tasks":preview["tasks"]
            }),
        )
        .await,
    );
    assert_eq!(preview_from_browser_link["id"], preview["id"]);
    let overview = result(
        lead.tool("plane_search_issues", json!({"project":"Billing"}))
            .await,
    );
    assert_eq!(overview["items"].as_array().unwrap().len(), 2);
    assert_eq!(overview["items"][0]["state"], "In progress");
    assert!(overview["project_errors"].as_array().unwrap().is_empty());
    let teammate = result(
        lead.tool("plane_search_issues", json!({"assignee":"Bob"}))
            .await,
    );
    assert_eq!(teammate["items"].as_array().unwrap().len(), 1);
    assert_eq!(teammate["items"][0]["url"], second);
    let mine = result(lead.tool("plane_search_issues", json!({"mine":true})).await);
    assert_eq!(mine["items"][0]["title"], "Implement");
    let named = result(
        lead.tool("plane_search_issues", json!({"query":"Verify"}))
            .await,
    );
    assert_eq!(named["items"][0]["url"], second);
    let limited = result(lead.tool("plane_search_issues", json!({"limit":1})).await);
    assert_eq!(limited["truncated"], true);
    let checkpoint = json!({"issue":second,"summary":"Validated schema","checks":["Integration passed"],
        "next":["Review"],"confirmed":true});
    let saved = result(lead.tool("plane_checkpoint", checkpoint.clone()).await);
    assert_eq!(
        result(lead.tool("plane_checkpoint", checkpoint).await)["comment_id"],
        saved["comment_id"]
    );
    fixture.lock().unwrap().changed = true;
    let mut member = start_mcp(&base, &plane_token, Some(&gitea_token), false);
    member
        .request("initialize", json!({"protocolVersion":"2025-11-25"}))
        .await;
    let tools = member.request("tools/list", json!({})).await;
    assert!(
        !tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "plane_apply_plan")
    );
    let context = result(
        member
            .tool("plane_issue_context", json!({"issue":second}))
            .await,
    );
    assert_eq!(context["issue"]["assignees"][0], BOB);
    assert_eq!(context["state_details"]["name"], "In progress");
    assert_eq!(
        context["relations"]["blocked_by"].as_array().unwrap().len(),
        1
    );
    assert_eq!(context["dependencies"][0]["status"], "needs_review");
    assert!(
        context["comments"][0]["comment_html"]
            .as_str()
            .unwrap()
            .contains("Validated schema")
    );
    assert_eq!(
        member.tool("plane_checkpoint", json!({})).await["isError"],
        true
    );
    assert_eq!(
        member
            .tool(
                "plane_issue_context",
                json!({"issue":parent.replace("127.0.0.1","other.invalid")})
            )
            .await["isError"],
        true
    );
    fixture.lock().unwrap().repeat_cursor = true;
    assert_eq!(
        member
            .tool("plane_issue_context", json!({"issue":parent}))
            .await["isError"],
        true
    );
    server.abort();
}
