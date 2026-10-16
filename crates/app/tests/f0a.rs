//! Slice F0a: the route1 Linux recorder is deterministic. `refs/interactions/f0a.json` holds two
//! recordings each of `01-sidebar-row-hover` and `09-sidebar-resize-drag` (tools/route1/linux,
//! `node lib.mjs record`); the two runs must match once timestamps are dropped, and the facts
//! the port's flows compare against must be in them.

use serde_json::Value;

fn strip_times(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("t");
            m.values_mut().for_each(strip_times);
        }
        Value::Array(a) => a.iter_mut().for_each(strip_times),
        _ => {}
    }
}

fn load() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../refs/interactions/f0a.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("f0a.json")).expect("valid JSON")
}

#[test]
fn two_recordings_match_after_dropping_timestamps() {
    let doc = load();
    let mut runs = doc["runs"].as_array().expect("runs").clone();
    assert_eq!(runs.len(), 2);
    runs.iter_mut().for_each(strip_times);
    for (a, b) in runs[0]
        .as_array()
        .unwrap()
        .iter()
        .zip(runs[1].as_array().unwrap())
    {
        assert_eq!(a["name"], b["name"]);
        assert_eq!(a, b, "{} differs", a["name"]);
    }
}

#[test]
fn recordings_hold_the_original_facts() {
    let doc = load();
    let [hover, resize] = [&doc["runs"][0][0], &doc["runs"][0][1]];
    let dom = |rec: &Value, i: usize| rec["result"]["dom"][i].clone();
    // 01: the hovered row fills rgba(255,255,255,0.08); the pin/archive group fades in.
    assert_eq!(
        dom(hover, 0)["computed"]["backgroundColor"],
        "rgba(0, 0, 0, 0)"
    );
    assert_eq!(
        dom(hover, 1)["computed"]["backgroundColor"],
        "rgba(255, 255, 255, 0.08)"
    );
    assert_eq!(dom(hover, 2)["opacity"], "1");
    // 09: the edge clamps at 290 (handle x 282) and 520 (handle x 512); a far drag collapses.
    let handle_x = |name: &str, pointer: u64| {
        let samples = resize["result"]["dom"]
            .as_array()
            .unwrap()
            .iter()
            .find(|d| d["selector"] == name)
            .unwrap()["samples"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["pointerX"] == pointer)
            .unwrap()
            .clone();
        samples["handleX"].as_u64().unwrap()
    };
    assert_eq!(handle_x("min / collapse", 200), 282);
    assert_eq!(handle_x("min / collapse", 110), 44);
    assert_eq!(handle_x("max", 900), 512);
}
