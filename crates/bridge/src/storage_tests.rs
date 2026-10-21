use super::*;
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;

fn thread(id: &str, parent: Option<&str>, updated: i64) -> Entry {
    Entry {
        archived: true,
        thread: json!({"id":id,"name":id,"source":if let Some(parent)=parent { json!({"subAgent":{"thread_spawn":{"parent_thread_id":parent}}}) } else { json!("cli") },"threadSource":"user","status":{"type":"idle"},"updatedAt":updated,"createdAt":1,"ephemeral":false}),
    }
}

fn usage() -> Usage {
    Usage {
        thread_bytes: Arc::new(HashMap::from([("root".into(), 20), ("child".into(), 10)])),
        ..Default::default()
    }
}

fn server(mut reply: impl FnMut(&Value) -> Value + Send + 'static) -> Bridge {
    let (read, mut output) = std::io::pipe().unwrap();
    let (input, write) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let mut initialized = false;
        for line in BufReader::new(input).lines() {
            let request: Value = serde_json::from_str(&line.unwrap()).unwrap();
            if request["id"].is_null() {
                continue;
            }
            let result = if request["method"] == "initialize" {
                assert!(!initialized, "initialize must not be sent twice");
                initialized = true;
                json!({"userAgent":"codex_cli_rs/0.163.0-alpha.10.2","codexHome":"/unused","platformOs":"linux"})
            } else {
                reply(&request)
            };
            if writeln!(output, "{}", json!({"id":request["id"],"result":result})).is_err() {
                break;
            }
        }
    });
    Bridge::from_io(read, write, None).0
}

#[test]
fn cleanup_requires_a_complete_inactive_old_tree() {
    let entries = vec![thread("root", None, 10), thread("child", Some("root"), 20)];
    let allowed = candidates(&entries, &usage(), 30, true);
    assert_eq!(allowed.len(), 1);
    assert_eq!(allowed[0].bytes, 30);
    assert_eq!(allowed[0].descendants[0].id(), "child");
    for mutation in [
        "active",
        "recent",
        "ephemeral",
        "missing-size",
        "unsupported-root",
        "cycle",
    ] {
        let mut entries = entries.clone();
        let mut usage = usage();
        match mutation {
            "active" => entries[1].thread["status"]["type"] = json!("active"),
            "recent" => entries[1].thread["updatedAt"] = json!(30),
            "ephemeral" => entries[1].thread["ephemeral"] = json!(true),
            "missing-size" => {
                Arc::make_mut(&mut usage.thread_bytes).remove("child");
            }
            "unsupported-root" => entries[0].thread["threadSource"] = json!("cloud_threads.create"),
            "cycle" => entries[0].thread["parentThreadId"] = json!("child"),
            _ => unreachable!(),
        }
        assert!(
            candidates(&entries, &usage, 30, true).is_empty(),
            "{mutation}"
        );
    }
}

#[test]
fn changed_descendants_prevent_deletion() {
    for changed in [false, true] {
        let deleted = Arc::new(Mutex::new(Vec::new()));
        let observed = deleted.clone();
        let old = vec![thread("root", None, 10), thread("child", Some("root"), 20)];
        let current = old.clone();
        let bridge = server(move |request| match request["method"].as_str().unwrap() {
            "thread/list" => {
                let data = if !request["params"]["parentThreadId"].is_null() {
                    if request["params"]["parentThreadId"] == "root" {
                        let mut child = current[1].thread.clone();
                        if changed {
                            child["updatedAt"] = json!(21);
                        }
                        vec![child]
                    } else {
                        vec![]
                    }
                } else if request["params"]["archived"] == true {
                    current.iter().map(|e| e.thread.clone()).collect()
                } else {
                    vec![]
                };
                json!({"data":data,"nextCursor":null})
            }
            "thread/read" => {
                json!({"thread":current.iter().find(|e|e.thread["id"] == request["params"]["threadId"]).unwrap().thread})
            }
            "thread/delete" => {
                observed
                    .lock()
                    .unwrap()
                    .push(request["params"]["threadId"].clone());
                json!({})
            }
            method => panic!("unexpected method {method}"),
        });
        let reviewed = candidates(&old, &usage(), 30, true);
        let result = futures_lite::future::block_on(delete_selected(
            &bridge,
            &reviewed,
            &HashSet::from(["root".into()]),
            &usage(),
            30,
            true,
        ))
        .unwrap();
        assert_eq!(result.deleted, usize::from(!changed));
        assert_eq!(result.skipped, usize::from(changed));
        assert_eq!(
            *deleted.lock().unwrap(),
            if changed { vec![] } else { vec![json!("root")] }
        );
    }
}

#[test]
fn measurement_counts_files_without_following_symlinks() {
    let root = std::env::temp_dir().join(format!(
        "cgx-storage-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(root.join("sessions")).unwrap();
    fs::create_dir_all(root.join("archived_sessions")).unwrap();
    fs::create_dir_all(root.join("worktrees")).unwrap();
    let id = "11111111-1111-4111-8111-111111111111";
    let mut main = thread(id, None, 10);
    main.thread["path"] = json!(root.join("sessions/legacy.jsonl"));
    fs::write(root.join("sessions/legacy.jsonl"), b"1234567").unwrap();
    fs::write(root.join("sessions/unclassified.jsonl"), b"123").unwrap();
    fs::write(root.join("archived_sessions/archive.jsonl"), b"12345").unwrap();
    fs::write(root.join("worktrees/file"), b"123456789").unwrap();
    fs::write(root.join("settings.json"), b"12").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("settings.json"), root.join("sessions/link.jsonl"))
        .unwrap();
    let bridge = server(move |request| match request["method"].as_str().unwrap() {
        "thread/list" => json!({"data":[main.thread.clone()],"nextCursor":null}),
        method => panic!("unexpected method {method}"),
    });
    let result = futures_lite::future::block_on(measure(&bridge, &root, |_, _| {})).unwrap();
    assert_eq!(result.bytes, [7, 0, 5, 3, 9, 2]);
    assert_eq!(result.total(), 26);
    assert_eq!(result.thread_bytes.get(id), Some(&7));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_version_respects_the_descendant_discovery_boundary() {
    for version in [
        "0.143.0-alpha.16",
        "0.143.0-alpha.16.1",
        "0.143.0-beta.1",
        "0.143.0",
        "0.163.0-alpha.10.2",
    ] {
        assert!(cleanup_version(version), "{version}");
    }
    for version in [
        "",
        "garbage",
        "0.142.9",
        "0.143.0-alpha.15",
        "0.143.0-alpha",
        "0.01.0",
    ] {
        assert!(!cleanup_version(version), "{version}");
    }
}

#[test]
fn cleanup_reuses_the_initialized_connection() {
    let bridge = server(|request| panic!("unexpected request {request}"));
    futures_lite::future::block_on(async {
        bridge.initialize("codex-gpui", "test").await.unwrap();
        assert!(supports_cleanup(&bridge).await);
        assert!(supports_cleanup(&bridge).await);
    });
}
