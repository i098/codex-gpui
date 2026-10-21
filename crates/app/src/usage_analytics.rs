use super::{http, load};
use crate::host::{Host, Identity};
use anyhow::{Context as _, Result, ensure};
use bridge::Bridge;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use ui::screens::settings::usage::analytics::{Metric, Query, Snapshot};

fn dates(now: Option<i64>, days: i64, offset: i64) -> Result<(String, String)> {
    ensure!(
        matches!(days, 7 | 30) && offset >= 0,
        "Invalid analytics period"
    );
    let now =
        chrono::DateTime::from_timestamp(now.unwrap_or_else(|| chrono::Utc::now().timestamp()), 0)
            .context("Invalid analytics date")?
            .date_naive();
    let offset = chrono::Duration::try_days(offset).context("Invalid analytics offset")?;
    let end = now
        .checked_sub_signed(offset)
        .context("Invalid analytics end date")?;
    let start = end
        .checked_sub_signed(chrono::Duration::days(days - 1))
        .context("Invalid analytics start date")?;
    Ok((start.to_string(), end.to_string()))
}

fn read(host: &dyn Host, identity: &Identity, query: Query, now: Option<i64>) -> Result<Value> {
    let (start, end) = dates(now, query.days, query.offset_days)?;
    let endpoint = match query.metric {
        Metric::Usage => "/wham/usage/daily-token-usage-breakdown",
        Metric::Credits => "/wham/usage/credit-usage-events",
        Metric::Plugins => "/wham/analytics/daily-plugin-usage-metrics",
        Metric::Skills => "/wham/analytics/daily-skill-usage-metrics",
        Metric::Messages => "/wham/analytics/daily-workspace-usage-counts",
        Metric::Plan => "/wham/usage/plan_limit_history",
        Metric::Chats => anyhow::bail!("Chat usage requires local thread identities"),
    };
    let mut url = url::Url::parse(&format!("https://chatgpt.com{endpoint}"))?;
    match query.metric {
        Metric::Plan => {
            url.query_pairs_mut().append_pair("days", "7");
        }
        Metric::Credits => {}
        _ => {
            url.query_pairs_mut()
                .append_pair("start_date", &start)
                .append_pair("end_date", &end)
                .append_pair("group_by", "day");
            match query.metric {
                Metric::Plugins => {
                    url.query_pairs_mut()
                        .append_pair("top_plugin_limit", "10")
                        .append_pair("workspace_user", "true");
                }
                Metric::Skills => {
                    url.query_pairs_mut()
                        .append_pair("top_skill_limit", "10")
                        .append_pair("workspace_user", "true");
                }
                Metric::Messages => {
                    url.query_pairs_mut().append_pair("workspace_user", "true");
                }
                _ => {}
            }
        }
    }
    let path = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().into(),
    };
    let result = http(host, identity, "GET", &path, None);
    let mut value = match result {
        Err(error)
            if query.metric == Metric::Plan && error.to_string() == "host HTTP status 404" =>
        {
            return Ok(Value::Null);
        }
        result => result?,
    };
    if query.metric == Metric::Credits {
        let rows = value["data"]
            .as_array_mut()
            .context("Credit history has no data")?;
        rows.retain(|row| {
            row["date"]
                .as_str()
                .is_some_and(|date| date >= start.as_str() && date <= end.as_str())
        });
    } else if query.metric != Metric::Plan {
        ensure!(value["data"].is_array(), "Analytics response has no data");
    }
    Ok(value)
}

async fn chats(
    bridge: &Bridge,
    host: &dyn Host,
    identity: &Identity,
    now: Option<i64>,
) -> Result<Value> {
    let threads: Value = bridge
        .request(
            "thread/list",
            json!({"limit":100,"sortKey":"updated_at","sortDirection":"desc","archived":false}),
        )
        .await?;
    let cutoff = now.unwrap_or_else(|| chrono::Utc::now().timestamp()) - 30 * 86400;
    let roots: Vec<_> = threads["data"]
        .as_array()
        .context("Thread list has no data")?
        .iter()
        .filter(|thread| {
            thread["updatedAt"]
                .as_i64()
                .is_some_and(|updated| updated >= cutoff)
                && thread["parentThreadId"].is_null()
        })
        .take(100)
        .cloned()
        .collect();
    if roots.is_empty() {
        return Ok(json!({"rows":[]}));
    }
    let mut batches = Vec::new();
    let mut batch = Vec::new();
    let mut count = 0;
    let mut seen = HashSet::new();
    for thread in &roots {
        let id = thread["id"].as_str().context("Thread has no id")?;
        let descendants: Value = bridge
            .request("thread/list", json!({"ancestorThreadId":id,"limit":1000}))
            .await?;
        ensure!(
            descendants["nextCursor"].is_null(),
            "Thread descendant list is incomplete"
        );
        let descendants: Vec<_> = descendants["data"]
            .as_array()
            .context("Thread descendants have no data")?
            .iter()
            .filter_map(|thread| thread["id"].as_str())
            .filter(|child| *child != id)
            .collect();
        ensure!(
            descendants.len() < 1000,
            "Thread descendant list exceeds the analytics limit"
        );
        ensure!(
            seen.insert(id.to_owned())
                && descendants.iter().all(|id| seen.insert((*id).to_owned())),
            "Thread usage groups overlap"
        );
        if batch.len() == 100 || count + 1 + descendants.len() > 1000 {
            batches.push(std::mem::take(&mut batch));
            count = 0;
        }
        let created = thread["createdAt"]
            .as_i64()
            .and_then(|time| chrono::DateTime::from_timestamp(time, 0))
            .map(|time| time.to_rfc3339());
        batch
            .push(json!({"thread_id":id,"created_at":created,"descendant_thread_ids":descendants}));
        count += 1 + descendants.len();
    }
    if !batch.is_empty() {
        batches.push(batch);
    }
    let mut usage = HashMap::new();
    for threads in batches {
        let response = http(
            host,
            identity,
            "POST",
            "/wham/usage/thread_usage/query_v2",
            Some(json!({"threads":threads})),
        )?;
        for row in response["threads"]
            .as_array()
            .context("Thread usage response has no threads")?
        {
            let id = row["thread_id"]
                .as_str()
                .context("Thread usage has no id")?;
            usage.insert(id.to_owned(), row.clone());
        }
    }
    Ok(json!({"rows":roots.into_iter().map(|thread| {
        let data = thread["id"].as_str().and_then(|id| usage.remove(id));
        json!({"thread":thread,"usage":data})
    }).collect::<Vec<_>>()}))
}

pub(super) async fn query(
    bridge: &Bridge,
    host: &dyn Host,
    identity: &Identity,
    request: Query,
    now: Option<i64>,
    mut snapshot: Snapshot,
) -> Snapshot {
    let value = if request.metric == Metric::Chats {
        chats(bridge, host, identity, now).await
    } else {
        read(host, identity, request, now)
    };
    snapshot.set(request.metric, load(value));
    if request.metric == Metric::Plugins {
        snapshot.set(
            Metric::Skills,
            load(read(
                host,
                identity,
                Query {
                    metric: Metric::Skills,
                    ..request
                },
                now,
            )),
        );
    }
    snapshot
}

pub(super) async fn snapshot(
    bridge: &Bridge,
    host: &dyn Host,
    identity: &Identity,
    now: Option<i64>,
) -> Snapshot {
    let mut snapshot = Snapshot {
        token_usage: load(
            bridge
                .request::<_, Value>("account/usage/read", json!({}))
                .await,
        ),
        ..Default::default()
    };
    for metric in [
        Metric::Usage,
        Metric::Chats,
        Metric::Plan,
        Metric::Credits,
        Metric::Plugins,
        Metric::Messages,
    ] {
        snapshot = query(
            bridge,
            host,
            identity,
            Query {
                metric,
                days: 7,
                offset_days: 0,
            },
            now,
            snapshot,
        )
        .await;
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_windows_cross_years_without_overlap() -> Result<()> {
        let now = chrono::DateTime::parse_from_rfc3339("2026-01-01T23:00:00Z")?.timestamp();
        assert_eq!(
            dates(Some(now), 7, 0)?,
            ("2025-12-26".into(), "2026-01-01".into())
        );
        assert_eq!(
            dates(Some(now), 7, 7)?,
            ("2025-12-19".into(), "2025-12-25".into())
        );
        assert!(dates(Some(now), 0, 0).is_err());
        assert!(dates(Some(now), 7, -1).is_err());
        Ok(())
    }
}
