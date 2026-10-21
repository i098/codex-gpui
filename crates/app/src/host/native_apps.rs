use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashSet, path::Path, process::Command};

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeApp {
    pub app_path: String,
    pub bundle_id: String,
    pub display_name: String,
}

struct Record {
    app: NativeApp,
    last_used: i64,
    use_count: u64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Order {
    Usage,
    DisplayName,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Query {
    last_used_within_days: Option<f64>,
    order: Option<Order>,
}

const EXCLUDED: [&str; 3] = [
    "com.apple.finder.Open-Computer",
    "com.openai.sky.CUAService",
    "com.spotify.installer",
];

pub(super) fn list(params: &Value) -> Result<Vec<NativeApp>> {
    let query: Query = serde_json::from_value(params.clone())?;
    if let Some(days) = query.last_used_within_days {
        ensure!(days.is_finite() && days >= 0., "invalid lastUsedWithinDays");
    }
    if !cfg!(target_os = "macos") {
        return Ok(Vec::new());
    }
    let output = Command::new("/usr/bin/mdfind")
        .args([
            "-0",
            "kMDItemContentType == \"com.apple.application-bundle\" && kMDItemFSName == \"*.app\"",
            "-attr",
            "kMDItemDisplayName",
            "-attr",
            "kMDItemCFBundleIdentifier",
            "-attr",
            "kMDItemLastUsedDate_Ranking",
            "-attr",
            "kMDItemUseCount",
        ])
        .output()
        .context("read native desktop applications")?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let now = chrono::Utc::now().timestamp_millis();
    Ok(select(&String::from_utf8_lossy(&output.stdout), query, now))
}

pub(super) fn by_bundle_id(params: &Value) -> Result<Option<NativeApp>> {
    let bundle_id = params["bundleId"].as_str().context("missing bundleId")?;
    if !is_literal_bundle_id(bundle_id) || !cfg!(target_os = "macos") {
        return Ok(None);
    }
    let query = format!("kMDItemCFBundleIdentifier == \"{bundle_id}\"");
    let output = Command::new("/usr/bin/mdfind")
        .arg(query)
        .output()
        .context("find native application by bundle ID")?;
    if !output.status.success() {
        return Ok(None);
    }
    let paths = String::from_utf8_lossy(&output.stdout);
    let app = paths
        .lines()
        .map(str::trim)
        .find(|path| path.ends_with(".app"))
        .map(|path| NativeApp {
            app_path: path.to_owned(),
            bundle_id: bundle_id.to_owned(),
            display_name: display_name(path),
        });
    Ok(app)
}

fn is_literal_bundle_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

pub(super) fn display_name(path: &str) -> String {
    let plist = Path::new(path).join("Contents/Info.plist");
    for key in ["CFBundleDisplayName", "CFBundleName"] {
        if let Ok(output) = Command::new("/usr/libexec/PlistBuddy")
            .args(["-c", &format!("Print :{key}")])
            .arg(&plist)
            .output()
            && output.status.success()
        {
            let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !name.is_empty() {
                return name.strip_suffix(".app").unwrap_or(&name).to_owned();
            }
        }
    }
    Path::new(path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

fn parse(record: &str) -> Option<Record> {
    let (path, rest) = record.split_once("kMDItemDisplayName = ")?;
    let (name, rest) = rest.split_once("kMDItemCFBundleIdentifier = ")?;
    let (bundle_id, rest) = rest.split_once("kMDItemLastUsedDate_Ranking = ")?;
    let (last_used, use_count) = rest.split_once("kMDItemUseCount = ")?;
    let path = path.trim();
    let bundle_id = bundle_id.trim();
    if !path.ends_with(".app")
        || bundle_id.is_empty()
        || bundle_id == "(null)"
        || EXCLUDED.contains(&bundle_id)
    {
        return None;
    }
    let name = name.trim();
    let name = if name == "(null)" {
        Path::new(path).file_name()?.to_str()?
    } else {
        name
    };
    Some(Record {
        app: NativeApp {
            app_path: path.to_owned(),
            bundle_id: bundle_id.to_owned(),
            display_name: name.strip_suffix(".app").unwrap_or(name).to_owned(),
        },
        last_used: chrono::DateTime::parse_from_str(last_used.trim(), "%Y-%m-%d %H:%M:%S%.f %z")
            .map(|date| date.timestamp_millis())
            .unwrap_or(0),
        use_count: use_count.trim().parse().unwrap_or(0),
    })
}

fn select(output: &str, query: Query, now: i64) -> Vec<NativeApp> {
    let cutoff = query
        .last_used_within_days
        .map(|days| now as f64 - days * 86_400_000.);
    let records = output
        .split('\0')
        .filter_map(parse)
        .filter(|record| cutoff.is_none_or(|cutoff| record.last_used as f64 >= cutoff))
        .collect();
    let mut seen = HashSet::new();
    sorted(records, query.order)
        .into_iter()
        .filter_map(|record| {
            seen.insert(record.app.app_path.clone())
                .then_some(record.app)
        })
        .collect()
}

fn usage_order(a: &Record, b: &Record, order: Order) -> std::cmp::Ordering {
    match order {
        Order::Usage => b
            .last_used
            .cmp(&a.last_used)
            .then(b.use_count.cmp(&a.use_count)),
        Order::DisplayName => std::cmp::Ordering::Equal,
    }
}

fn sorted(records: Vec<Record>, order: Option<Order>) -> Vec<Record> {
    let Some(order) = order else {
        return records;
    };
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSString;
        let mut keyed: Vec<_> = records
            .into_iter()
            .map(|record| {
                let name = NSString::from_str(&record.app.display_name);
                (record, name)
            })
            .collect();
        keyed.sort_by(|(a, a_name), (b, b_name)| {
            usage_order(a, b, order).then_with(|| a_name.localizedCompare(b_name).into())
        });
        keyed.into_iter().map(|(record, _)| record).collect()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut records = records;
        records.sort_by(|a, b| {
            usage_order(a, b, order).then_with(|| a.app.display_name.cmp(&b.app.display_name))
        });
        records
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_selection_filters_hidden_apps_and_keeps_usage_order() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let rows = [
            "/Applications/Old.app kMDItemDisplayName = Old kMDItemCFBundleIdentifier = org.example.old kMDItemLastUsedDate_Ranking = 2026-10-01 00:00:00 +0000 kMDItemUseCount = 500",
            "/Applications/Used.app kMDItemDisplayName = Used.app kMDItemCFBundleIdentifier = org.example.used kMDItemLastUsedDate_Ranking = 2026-10-20 00:00:00 +0000 kMDItemUseCount = 10",
            "/Applications/Hidden.app kMDItemDisplayName = Hidden kMDItemCFBundleIdentifier = com.openai.sky.CUAService kMDItemLastUsedDate_Ranking = 2026-10-20 00:00:00 +0000 kMDItemUseCount = 100",
            "/Applications/Other.app kMDItemDisplayName = (null) kMDItemCFBundleIdentifier = org.example.other kMDItemLastUsedDate_Ranking = 2026-10-20 00:00:00 +0000 kMDItemUseCount = 2",
        ].join("\0");
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-21T00:00:00Z")
            .unwrap()
            .timestamp_millis();
        let selected = select(
            &rows,
            Query {
                last_used_within_days: Some(7.),
                order: Some(Order::Usage),
            },
            now,
        );
        assert_eq!(
            selected
                .iter()
                .map(|app| app.display_name.as_str())
                .collect::<Vec<_>>(),
            ["Used", "Other"]
        );
        assert_eq!(selected[0].bundle_id, "org.example.used");
    }

    #[test]
    fn name_order_and_literal_bundle_ids_reject_query_patterns() {
        #[cfg(target_os = "macos")]
        std::panic::set_hook(Box::new(|info| {
            eprintln!("{info}");
            std::process::exit(101);
        }));
        let rows = concat!(
            "/Applications/Zebra.app kMDItemDisplayName = Zebra kMDItemCFBundleIdentifier = org.example.zebra kMDItemLastUsedDate_Ranking = (null) kMDItemUseCount = (null)",
            "\0",
            "/Applications/Alpha.app kMDItemDisplayName = Alpha kMDItemCFBundleIdentifier = org.example.alpha kMDItemLastUsedDate_Ranking = (null) kMDItemUseCount = 0"
        );
        let selected = select(
            rows,
            Query {
                last_used_within_days: None,
                order: Some(Order::DisplayName),
            },
            0,
        );
        assert_eq!(
            selected
                .iter()
                .map(|app| app.display_name.as_str())
                .collect::<Vec<_>>(),
            ["Alpha", "Zebra"]
        );
        assert!(list(&serde_json::json!({"lastUsedWithinDays": -1})).is_err());
        assert!(is_literal_bundle_id("org.example_app.2"));
        assert!(!is_literal_bundle_id("com.apple.*"));
        assert!(!is_literal_bundle_id("com.apple.?"));
        assert!(!is_literal_bundle_id("com.apple.Terminal\" || true"));
    }
}
