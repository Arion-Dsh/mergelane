use std::time::Duration;

use serde_json::{Value, json};
use tempfile::TempDir;

mod support;
#[path = "support/tracker_api.rs"]
mod tracker_api;

use support::{Mcp, token, tool_result};
use tracker_api::{Platform, Server};

fn start(platform: Platform, server: &Server, token: &std::path::Path, write: bool) -> Mcp {
    let name = platform.name();
    let mut command = support::command();
    command
        .args([
            "mcp",
            &format!("--{name}-url"),
            &server.base,
            &format!("--{name}-token-file"),
        ])
        .arg(token);
    if matches!(platform, Platform::Jira) {
        command.args(["--jira-email", "test@example.com"]);
    }
    if write {
        command.arg(format!("--{name}-write"));
    }
    Mcp::start(command)
}

async fn initialize(mcp: &mut Mcp) {
    mcp.request("initialize", json!({"protocolVersion":"2025-11-25"}))
        .await;
}

#[tokio::test]
async fn pull_loads_saved_accounts_and_reports_pagination_limits() {
    let github = Server::start(Platform::GitHub).await;
    let gitlab = Server::start(Platform::GitLab).await;
    let jira = Server::start(Platform::Jira).await;
    let home = TempDir::new().unwrap();
    token(home.path(), "token", "fake-token");
    let config = home.path().join("profile.json");
    std::fs::write(
        &config,
        json!({"version":1,"backends":{
            "github":{"url":github.base,"token_file":"token","write":true},
            "gitlab":{"url":gitlab.base,"token_file":"token"},
            "jira":{"url":jira.base,"token_file":"token","email":"test@example.com"}
        }})
        .to_string(),
    )
    .unwrap();
    let pull = |flags: &[&str]| {
        let mut command = support::command();
        command.arg("--config").arg(&config).arg("pull").args(flags);
        command
    };

    let output = pull(&[]).output().await.unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "assigned_to_me");
    assert_eq!(report["complete"], true);
    let sources = report["sources"].as_array().unwrap();
    assert_eq!(sources.len(), 3);
    for source in sources {
        let items = source["items"].as_array().unwrap();
        assert_eq!(
            items.len(),
            1,
            "duplicate issues across pages should collapse"
        );
        assert_eq!(items[0]["title"], "Billing export");
        assert!(items[0].get("description").is_none());
    }
    assert_eq!(github.fixture.lock().unwrap().requests, 4);
    assert_eq!(jira.fixture.lock().unwrap().requests, 2);

    let mut command = support::command();
    command
        .arg("--config")
        .arg(&config)
        .args(["mcp", "--read-only"]);
    let mut mcp = Mcp::start(command);
    initialize(&mut mcp).await;
    let tools = mcp.request("tools/list", json!({})).await;
    let pull_tool = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "mergelane_pull_tasks")
        .unwrap();
    assert_eq!(pull_tool["annotations"]["readOnlyHint"], true);
    let via_mcp = tool_result(mcp.tool("mergelane_pull_tasks", json!({})).await);
    assert_eq!(via_mcp["scope"], report["scope"]);
    assert_eq!(via_mcp["complete"], report["complete"]);
    assert_eq!(via_mcp["sources"], report["sources"]);

    let output = pull(&[
        "--backend",
        "github",
        "--project",
        "team/web",
        "--max-pages",
        "1",
    ])
    .output()
    .await
    .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["scope"], "project");
    assert_eq!(report["complete"], false);
    assert_eq!(report["sources"].as_array().unwrap().len(), 1);
    assert_eq!(report["sources"][0]["truncated"], true);
    assert_eq!(report["sources"][0]["items"][0]["title"], "Billing export");
    let via_mcp = tool_result(
        mcp.tool(
            "mergelane_pull_tasks",
            json!({"backend":"github","project":"team/web","max_pages":1}),
        )
        .await,
    );
    assert_eq!(via_mcp["scope"], report["scope"]);
    assert_eq!(via_mcp["project"], report["project"]);
    assert_eq!(via_mcp["complete"], report["complete"]);
    assert_eq!(via_mcp["sources"], report["sources"]);
    for invalid in [json!({"project":"team/web"}), json!({"max_pages":35})] {
        assert_eq!(
            mcp.tool("mergelane_pull_tasks", invalid).await["isError"],
            true
        );
    }
    for server in [&github, &gitlab, &jira] {
        assert_eq!(server.fixture.lock().unwrap().counts(), (0, 0, 0));
    }
}

#[tokio::test]
async fn local_lease_serializes_processes_and_expires_after_crash() {
    let server = Server::start(Platform::GitHub).await;
    let home = TempDir::new().unwrap();
    let other_worktree = TempDir::new().unwrap();
    let credential = token(home.path(), "token", "fake-token");
    let launch = |cwd: &std::path::Path| {
        let mut command = support::command();
        command
            .current_dir(cwd)
            .env(
                if cfg!(windows) { "USERPROFILE" } else { "HOME" },
                home.path(),
            )
            .args([
                "--no-config",
                "mcp",
                "--github-url",
                &server.base,
                "--github-token-file",
            ])
            .arg(&credential)
            .args(["--github-write", "--lease-seconds", "3"]);
        Mcp::start(command)
    };
    let mut first = launch(home.path());
    let mut second = launch(other_worktree.path());
    initialize(&mut first).await;
    initialize(&mut second).await;
    let short = json!({"backend":"github","issue":"team/web#42"});
    let link = json!({"backend":"github","issue":format!("{}/team/web/issues/42", server.base)});
    let (a, b) = tokio::join!(
        first.tool("mergelane_claim_task", short.clone()),
        second.tool("mergelane_claim_task", link.clone())
    );
    let (winner, other, result) = match (a["isError"] == true, b["isError"] == true) {
        (false, true) => (&mut first, &mut second, a),
        (true, false) => (&mut second, &mut first, b),
        _ => panic!("expected one claim winner: {a}, {b}"),
    };
    let grant = tool_result(result);
    let id = json!({"lease_id": grant["lease_id"]});
    assert_eq!(
        other.tool("mergelane_release_task", id.clone()).await["isError"],
        true
    );
    assert_eq!(
        other.tool("mergelane_renew_task", id.clone()).await["isError"],
        true
    );
    let checkpoint = json!({
        "issue":"team/web#42", "summary":"Test progress", "next":[], "confirmed":true,
        "lease_id":grant["lease_id"]
    });
    assert_eq!(
        other.tool("github_checkpoint", checkpoint.clone()).await["isError"],
        true
    );
    let mut unleased = checkpoint.clone();
    unleased.as_object_mut().unwrap().remove("lease_id");
    assert_eq!(
        other.tool("github_checkpoint", unleased.clone()).await["isError"],
        true
    );
    tool_result(winner.tool("github_checkpoint", checkpoint.clone()).await);

    let renewed = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let status = other.tool("mergelane_task_lease", short.clone()).await;
            if status["isError"] != true {
                let current = tool_result(status);
                if current["expires_at_unix_ms"].as_u64() > grant["expires_at_unix_ms"].as_u64() {
                    break current;
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .expect("owning MCP renews without further calls");
    assert_eq!(renewed["status"], "held");

    winner.child.kill().await.unwrap();
    assert_eq!(
        other.tool("mergelane_claim_task", link.clone()).await["isError"],
        true
    );
    let takeover = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let response = other.tool("mergelane_claim_task", link.clone()).await;
            if response["isError"] != true {
                break tool_result(response);
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .expect("dead session's lease expires without renewal");
    assert_ne!(takeover["lease_id"], grant["lease_id"]);
    assert_eq!(
        other.tool("github_checkpoint", checkpoint).await["isError"],
        true
    );
    assert_eq!(
        tool_result(
            other
                .tool(
                    "mergelane_release_task",
                    json!({
                        "lease_id": takeover["lease_id"]
                    })
                )
                .await
        )["status"],
        "released"
    );
    assert_eq!(
        other.tool("github_checkpoint", unleased).await["isError"],
        true
    );
    let mut reopened = launch(home.path());
    initialize(&mut reopened).await;
    let next: Value = tool_result(reopened.tool("mergelane_claim_task", short).await);
    assert_ne!(next["lease_id"], takeover["lease_id"]);
}

async fn workflow(platform: Platform) {
    let server = Server::start(platform).await;
    let dir = TempDir::new().unwrap();
    let token = token(dir.path(), "token", "fake-token");
    let name = platform.name();
    let mut lead = start(platform, &server, &token, true);
    initialize(&mut lead).await;
    assert_eq!(
        lead.request("tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .len(),
        13
    );
    let parent = platform.key(42);
    let context = tool_result(
        lead.tool(&format!("{name}_issue_context"), json!({"issue":parent}))
            .await,
    );
    assert_eq!(context["members"].as_array().unwrap().len(), 2);
    let draft = json!({"parent":parent,"tasks":[
        {"title":"Implement export","assignee":platform.assignee(false),"acceptance":["Tests pass"]},
        {"title":"Verify export","assignee":platform.assignee(true),"acceptance":["Output matches spec"],"after":[0]}
    ]});
    let preview = tool_result(
        lead.tool(&format!("{name}_preview_plan"), draft.clone())
            .await,
    );
    assert_eq!(server.fixture.lock().unwrap().counts(), (0, 0, 0));
    let apply = format!("{name}_apply_plan");
    assert_eq!(
        lead.tool(&apply, json!({"plan_id":preview["id"],"confirmed":false}))
            .await["isError"],
        true
    );
    assert_eq!(
        lead.tool(&apply, json!({"plan_id":"stale","confirmed":true}))
            .await["isError"],
        true
    );
    let approved = json!({"plan_id":preview["id"],"confirmed":true});
    assert_eq!(lead.tool(&apply, approved.clone()).await["isError"], true);
    assert_eq!(
        server.fixture.lock().unwrap().counts(),
        (2, 0, 0),
        "second task persisted despite an error response"
    );
    lead.child.kill().await.unwrap();
    let mut lead = start(platform, &server, &token, true);
    initialize(&mut lead).await;
    assert_eq!(
        lead.tool(&apply, approved.clone()).await["isError"],
        true,
        "preview is scoped to the process"
    );
    let repeated = tool_result(lead.tool(&format!("{name}_preview_plan"), draft).await);
    assert_eq!(preview["id"], repeated["id"]);
    assert_eq!(
        lead.tool(&apply, approved.clone()).await["isError"],
        true,
        "first native link write fails"
    );
    assert_eq!(server.fixture.lock().unwrap().counts(), (2, 0, 0));
    assert_eq!(
        lead.tool(&apply, approved.clone()).await["isError"],
        true,
        "summary persists before an error response"
    );
    assert_eq!(server.fixture.lock().unwrap().counts(), (2, 1, 1));
    let published = tool_result(lead.tool(&apply, approved.clone()).await);
    assert_eq!(published["issues"].as_array().unwrap().len(), 2);
    tool_result(lead.tool(&apply, approved).await);
    assert_eq!(
        server.fixture.lock().unwrap().counts(),
        (2, 1, 1),
        "retry must not duplicate tasks, links or summaries"
    );
    let checkpoint = json!({"issue":platform.key(44),"summary":"Export ready for review","checks":["Tests pass"],"next":["Review pull request"],"confirmed":true});
    let checkpoint_tool = format!("{name}_checkpoint");
    tool_result(lead.tool(&checkpoint_tool, checkpoint.clone()).await);
    tool_result(lead.tool(&checkpoint_tool, checkpoint).await);
    assert_eq!(
        server.fixture.lock().unwrap().counts(),
        (2, 1, 2),
        "checkpoints deduplicate retries"
    );
    let mut member = start(platform, &server, &token, false);
    initialize(&mut member).await;
    assert_eq!(
        member.request("tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert_eq!(
        member
            .tool(&apply, json!({"plan_id":preview["id"],"confirmed":true}))
            .await["isError"],
        true
    );
    let context = tool_result(
        member
            .tool(
                &format!("{name}_issue_context"),
                json!({"issue":platform.key(44)}),
            )
            .await,
    );
    assert_eq!(context["blocking_issues"][0], platform.key(43));
    assert!(
        context["comments"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Export ready for review")
    );
    let search = tool_result(
        member
            .tool(
                &format!("{name}_search_issues"),
                json!({"project":platform.project(),"query":"export","mine":true}),
            )
            .await,
    );
    assert_eq!(search["items"].as_array().unwrap().len(), 2);
    let page = if matches!(platform, Platform::Jira) {
        json!({"project":platform.project(),"cursor":search["next_cursor"]})
    } else {
        json!({"project":platform.project(),"page":2})
    };
    assert!(
        tool_result(member.tool(&format!("{name}_search_issues"), page).await)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let before = server.fixture.lock().unwrap().requests;
    let bad = format!("https://other.invalid/{name}/browse/WEB-42");
    assert_eq!(
        member
            .tool(&format!("{name}_issue_context"), json!({"issue":bad}))
            .await["isError"],
        true
    );
    assert_eq!(
        server.fixture.lock().unwrap().requests,
        before,
        "reject foreign URLs before sending credentials"
    );
}

#[tokio::test]
async fn github_shares_tasks_recovers_partial_writes_and_checks_dependencies() {
    workflow(Platform::GitHub).await;
}

#[tokio::test]
async fn gitlab_shares_tasks_recovers_partial_writes_and_checks_dependencies() {
    workflow(Platform::GitLab).await;
}

#[tokio::test]
async fn jira_shares_tasks_recovers_partial_writes_and_checks_dependencies() {
    workflow(Platform::Jira).await;
}

#[tokio::test]
async fn unassigned_issues_can_be_created_from_a_saved_project_and_assigned_in_another_session() {
    for platform in [Platform::GitHub, Platform::GitLab, Platform::Jira] {
        let server = Server::start(platform).await;
        let dir = TempDir::new().unwrap();
        let creator_token = token(dir.path(), "creator", "fake-token");
        let manager_token = token(dir.path(), "manager", "manager-token");
        let profile = dir.path().join("profile.json");
        let backend = platform.name();
        let mut init = support::command();
        init.args(["--config"])
            .arg(&profile)
            .args([
                "init",
                "--non-interactive",
                "--backend",
                backend,
                "--url",
                &server.base,
                "--default-project",
                platform.project(),
                "--write",
                "--token-file",
            ])
            .arg(&creator_token);
        if matches!(platform, Platform::Jira) {
            init.args(["--email", "test@example.com"]);
        }
        let output = init.output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let mut command = support::command();
        command.arg("mcp").arg("--config").arg(&profile);
        let mut creator = Mcp::start(command);
        initialize(&mut creator).await;
        let create = format!("{backend}_create_issue");
        let assign = format!("{backend}_assign_issue");
        let edit = format!("{backend}_edit_issue");
        let title = "Triage a billing request";
        assert_eq!(
            creator
                .tool(&create, json!({"title":title,"confirmed":false}))
                .await["isError"],
            true
        );
        assert_eq!(server.fixture.lock().unwrap().requests, 0);
        let created = tool_result(
            creator
                .tool(&create, json!({"title":title,"confirmed":true}))
                .await,
        );
        assert_eq!(created["issue"]["title"], title);
        assert_eq!(created["issue"]["description"], "");
        assert_eq!(created["issue"]["assignees"], json!([]));
        let url = created["url"].as_str().unwrap();
        drop(creator);

        let mut manager = start(platform, &server, &manager_token, true);
        initialize(&mut manager).await;
        let owner = platform.assignee(true);
        let browser_url = if matches!(platform, Platform::GitLab) {
            url.replace("/-/issues/", "/-/work_items/")
        } else {
            url.to_owned()
        };
        for reference in [browser_url.as_str(), url] {
            let assigned = tool_result(
                manager
                    .tool(
                        &assign,
                        json!({
                            "issue":reference, "assignee":owner, "confirmed":true
                        }),
                    )
                    .await,
            );
            assert_eq!(assigned["url"], url);
            assert_eq!(assigned["issue"]["assignees"][0]["id"], owner);
        }
        let conflict = manager
            .tool(
                &assign,
                json!({
                    "issue":url, "assignee":platform.assignee(false), "confirmed":true
                }),
            )
            .await;
        assert_eq!(conflict["isError"], true);
        assert!(
            conflict["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("already has an assignee")
        );

        let requests = server.fixture.lock().unwrap().requests;
        for invalid in [
            json!({"issue":url,"title":"Updated title","confirmed":false}),
            json!({"issue":url,"confirmed":true}),
            json!({"issue":url,"description":"mergelane:task:invalid:0","confirmed":true}),
        ] {
            assert_eq!(manager.tool(&edit, invalid).await["isError"], true);
        }
        let mut reader = start(platform, &server, &manager_token, false);
        initialize(&mut reader).await;
        assert_eq!(
            reader
                .tool(
                    &edit,
                    json!({"issue":url,"title":"Updated title","confirmed":true})
                )
                .await["isError"],
            true
        );
        assert_eq!(server.fixture.lock().unwrap().requests, requests);

        let description = "First line\n\nSecond <line>";
        let edited = tool_result(
            manager
                .tool(
                    &edit,
                    json!({
                        "issue":browser_url,"description":description,"confirmed":true
                    }),
                )
                .await,
        );
        assert_eq!(edited["url"], url);
        assert_eq!(edited["issue"]["title"], title);
        assert_eq!(edited["issue"]["description"], description);
        assert_eq!(edited["issue"]["assignees"][0]["id"], owner);

        let renamed = tool_result(
            manager
                .tool(
                    &edit,
                    json!({
                        "issue":url,"title":"Updated title","confirmed":true
                    }),
                )
                .await,
        );
        assert_eq!(renamed["issue"]["title"], "Updated title");
        assert_eq!(renamed["issue"]["description"], description);

        let cleared = tool_result(
            manager
                .tool(
                    &edit,
                    json!({
                        "issue":url,"description":"","confirmed":true
                    }),
                )
                .await,
        );
        assert_eq!(cleared["issue"]["description"], "");
        assert_eq!(cleared["issue"]["title"], "Updated title");
        assert_eq!(cleared["issue"]["assignees"][0]["id"], owner);
        assert_eq!(server.fixture.lock().unwrap().counts(), (1, 0, 0));
    }
}

#[tokio::test]
async fn all_five_backends_keep_tool_names_and_write_permissions_separate() {
    let dir = TempDir::new().unwrap();
    let token = token(dir.path(), "token", "fake-token");
    let mut command = support::command();
    command.arg("mcp");
    for backend in ["gitea", "plane", "github", "gitlab", "jira"] {
        command
            .args([
                format!("--{backend}-url"),
                format!("https://{backend}.example.com"),
            ])
            .arg(format!("--{backend}-token-file"))
            .arg(&token);
    }
    command.args([
        "--jira-email",
        "test@example.com",
        "--github-write",
        "--jira-write",
    ]);
    let mut client = Mcp::start(command);
    initialize(&mut client).await;
    let tools = client.request("tools/list", json!({})).await;
    let tools = tools["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 31);
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(names.len(), tools.len());
    assert!(names.contains("plane_preview_parent_checkpoint"));
    assert!(!names.contains("plane_publish_parent_checkpoint"));
    for backend in ["gitea", "plane", "github", "gitlab", "jira"] {
        for operation in ["apply_plan", "create_issue", "assign_issue", "edit_issue"] {
            let name = format!("{backend}_{operation}");
            assert_eq!(
                names.contains(name.as_str()),
                matches!(backend, "github" | "jira")
            );
        }
    }
    client.child.kill().await.unwrap();
}

#[tokio::test]
async fn setup_preserves_other_backends_and_doctor_only_reads_authenticated_accounts() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    let token_path = token(dir.path(), "token", "fake-token");
    let mut servers = Vec::new();
    for platform in [Platform::GitHub, Platform::GitLab, Platform::Jira] {
        let server = Server::start(platform).await;
        let mut command = support::command();
        command
            .arg("--config")
            .arg(&config)
            .args([
                "init",
                "--backend",
                platform.name(),
                "--url",
                &server.base,
                "--token-file",
            ])
            .arg(&token_path)
            .args(["--non-interactive", "--client", "cursor", "--write"]);
        if matches!(platform, Platform::Jira) {
            command.args(["--email", "test@example.com"]);
        }
        let output = command.output().await.unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let client: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(client["mcpServers"]["mergelane"]["args"][0], "mcp");
        assert_eq!(
            client["mcpServers"]["mergelane"]["args"][2],
            config.to_str().unwrap()
        );
        servers.push(server);
    }
    let saved = std::fs::read_to_string(&config).unwrap();
    assert!(
        !saved.contains("fake-token"),
        "only credential paths belong in settings"
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&saved).unwrap()["backends"]
            .as_object()
            .unwrap()
            .len(),
        3
    );
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .arg("doctor")
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["checks"].as_array().unwrap().len(), 3);
    for server in &servers {
        assert_eq!(server.fixture.lock().unwrap().counts(), (0, 0, 0));
    }
    let mut command = support::command();
    command
        .arg("mcp")
        .arg("--config")
        .arg(&config)
        .arg("--read-only");
    let mut client = Mcp::start(command);
    initialize(&mut client).await;
    assert_eq!(
        client.request("tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .len(),
        11
    );
    let context = tool_result(
        client
            .tool("github_issue_context", json!({"issue":"team/web#42"}))
            .await,
    );
    assert_eq!(context["issue"]["title"], "Billing export");
    client.child.kill().await.unwrap();
    let before: usize = servers
        .iter()
        .map(|server| server.fixture.lock().unwrap().requests)
        .sum();
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args(["doctor", "--offline"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let after: usize = servers
        .iter()
        .map(|server| server.fixture.lock().unwrap().requests)
        .sum();
    assert_eq!(before, after, "offline doctor must not send API requests");
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args([
            "init",
            "--backend",
            "github",
            "--url",
            &servers[0].base,
            "--token-file",
        ])
        .arg(&token_path)
        .arg("--non-interactive")
        .output()
        .await
        .unwrap();
    assert!(
        !output.status.success(),
        "existing backend requires --replace"
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), saved);
    let mut configured = support::command();
    configured.arg("mcp").arg("--config").arg(&config);
    let mut configured = Mcp::start(configured);
    initialize(&mut configured).await;
    assert_eq!(
        configured.request("tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .len(),
        29
    );
    configured.child.kill().await.unwrap();
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args([
            "doctor",
            "--offline",
            "--github-url",
            "https://override.example.com",
        ])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "explicit CLI values override saved arguments"
    );
    std::fs::write(&token_path, "invalid-fake-token").unwrap();
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .arg("doctor")
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert!(String::from_utf8_lossy(&output.stdout).contains("401"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("invalid-fake-token"));
}

#[tokio::test]
async fn invalid_settings_fail_without_overwriting_and_help_can_bypass_them() {
    let dir = TempDir::new().unwrap();
    let config = dir.path().join("config.json");
    std::fs::write(&config, r#"{"version":999,"backends":{}}"#).unwrap();
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args(["doctor", "--offline"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported"));
    let output = support::command()
        .arg("--config")
        .arg(&config)
        .args(["mcp", "--help"])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let mut command = support::command();
    command
        .args([
            "--no-config",
            "mcp",
            "--github-url",
            "https://github.com",
            "--github-token-file",
        ])
        .arg(token(dir.path(), "token", "fake-token"));
    let mut client = Mcp::start(command);
    initialize(&mut client).await;
    assert_eq!(
        client.request("tools/list", json!({})).await["tools"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    client.child.kill().await.unwrap();
}
