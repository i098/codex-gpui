use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

fn timestamp<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<i64>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::Number(number)) => number
            .as_i64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("Invalid expiry timestamp")),
        Some(serde_json::Value::String(value)) => chrono::DateTime::parse_from_rfc3339(&value)
            .map(|time| Some(time.timestamp()))
            .map_err(serde::de::Error::custom),
        _ => Err(serde::de::Error::custom("Invalid expiry timestamp")),
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub rate_limits: Limit,
    #[serde(default, deserialize_with = "null_default")]
    pub rate_limits_by_limit_id: BTreeMap<String, Limit>,
    pub rate_limit_reset_credits: Option<ResetCredits>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub limit_name: Option<String>,
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
    pub credits: Option<Credits>,
    pub plan_type: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub used_percent: f64,
    pub window_duration_mins: Option<i64>,
    pub resets_at: Option<i64>,
}

impl Window {
    pub fn remaining(&self) -> f64 {
        (100. - self.used_percent).clamp(0., 100.)
    }

    pub fn label(&self) -> &'static str {
        match self.window_duration_mins.unwrap_or(0) {
            285..=315 => "5-hour limit",
            9576..=10584 => "Weekly limit",
            1368..=1512 => "Daily usage limit",
            41040..=45360 => "Monthly usage limit",
            _ => "Usage limit",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Credits {
    pub balance: Option<String>,
    #[serde(default)]
    pub unlimited: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredits {
    #[serde(default, alias = "available_count")]
    pub available_count: usize,
    #[serde(default, deserialize_with = "null_default")]
    pub credits: Vec<ResetCredit>,
    #[serde(default, alias = "history_enabled")]
    pub history_enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetCredit {
    pub id: String,
    pub title: Option<String>,
    pub status: String,
    #[serde(default, alias = "expires_at", deserialize_with = "timestamp")]
    pub expires_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Reload {
    pub is_enabled: bool,
    pub recharge_threshold: Option<String>,
    pub recharge_target: Option<String>,
    pub recharge_monthly_limit: Option<String>,
    pub payment_method: Option<serde_json::Value>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ReloadDraft {
    pub threshold: String,
    pub target: String,
    pub monthly: String,
}

impl ReloadDraft {
    pub fn from_settings(settings: &Reload) -> Self {
        Self {
            threshold: settings
                .recharge_threshold
                .clone()
                .unwrap_or_else(|| "125".into()),
            target: settings
                .recharge_target
                .clone()
                .unwrap_or_else(|| "250".into()),
            monthly: settings.recharge_monthly_limit.clone().unwrap_or_default(),
        }
    }

    pub fn validate(&self, settings: &Reload, maximum: u64) -> Result<(), &'static str> {
        fn amount(text: &str) -> Option<u64> {
            let text = text.trim();
            (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| text.parse().ok())
                .flatten()
        }
        let threshold = amount(&self.threshold).ok_or("Minimum balance must be a whole number.")?;
        if threshold < 125 {
            return Err("Set the minimum balance to at least 125 credits.");
        }
        let target = amount(&self.target).ok_or("Target balance must be a whole number.")?;
        if target
            .checked_sub(threshold)
            .is_none_or(|difference| difference < 125)
        {
            return Err(
                "Set the target balance to at least 125 credits above the minimum balance.",
            );
        }
        if target > maximum
            && (!settings.is_enabled
                || settings.recharge_target.as_deref() != Some(self.target.trim()))
        {
            return Err("Target balance exceeds the maximum credit quantity.");
        }
        if !self.monthly.trim().is_empty()
            && amount(&self.monthly).is_none_or(|monthly| monthly < target)
        {
            return Err(
                "Monthly reload limit must be a whole number at least as large as the target balance.",
            );
        }
        Ok(())
    }

    pub fn payload(&self, settings: &Reload) -> serde_json::Value {
        let mut payload = serde_json::json!({
            "recharge_threshold": self.threshold.trim(),
            "recharge_target": self.target.trim(),
        });
        if !self.monthly.trim().is_empty() || settings.recharge_monthly_limit.is_some() {
            payload["recharge_monthly_limit"] = if self.monthly.trim().is_empty() {
                serde_json::Value::Null
            } else {
                self.monthly.trim().into()
            };
        }
        payload
    }
}

/// The original's warning after a failed initial auto-reload top-up, and whether the card was
/// declined (which offers the payment recovery links). `None` when the top-up did not fail.
pub fn top_up_warning(response: &serde_json::Value) -> Option<(&'static str, bool)> {
    let declined = match response["immediate_top_up_status"].as_str()? {
        "payment_declined" => true,
        "failed" => false,
        _ => return None,
    };
    let warning = match (declined, response["is_enabled"] == true) {
        (true, true) => {
            "Auto-reload is on, but the initial top-up was declined. Update your payment method before trying again."
        }
        (true, false) => {
            "Auto-reload is off. The initial top-up was declined. Update your payment method before trying again."
        }
        (false, true) => {
            "Auto-reload is on, but we couldn’t confirm the initial top-up. Check your transaction history before trying again."
        }
        (false, false) => {
            "Auto-reload is off, and we couldn’t confirm the initial top-up. Check your transaction history before trying again."
        }
    };
    Some((warning, declined))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BalanceHistory {
    #[serde(default)]
    pub points: Vec<BalancePoint>,
    pub data_as_of: Option<String>,
    pub next_cursor: Option<String>,
}

pub struct HistorySample {
    pub timestamp: i64,
    pub remaining: Option<f64>,
}

impl BalanceHistory {
    pub fn samples(&self, now: i64) -> Result<Vec<HistorySample>, String> {
        let parse = |value: &str| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|time| time.timestamp_millis())
                .map_err(|error| error.to_string())
        };
        let points: Vec<_> = self
            .points
            .iter()
            .map(|point| {
                Ok((
                    parse(&point.observed_at)?,
                    parse(&point.estimated_reset_at)?,
                    point,
                ))
            })
            .collect::<Result<_, String>>()?;
        let Some(&(start, _, _)) = points.first() else {
            return Ok(Vec::new());
        };
        if now < start || points.windows(2).any(|pair| pair[0].0 > pair[1].0) {
            return Err("Invalid plan history interval".into());
        }
        let cutoff = self
            .data_as_of
            .as_deref()
            .map(parse)
            .transpose()?
            .unwrap_or_else(|| points.last().map(|point| point.0).unwrap_or(start));
        let resets: Vec<_> = points
            .windows(2)
            .filter_map(|pair| {
                let (observed, reset, point) = pair[0];
                let (next_observed, next_reset, next) = pair[1];
                (point.plan_type == next.plan_type
                    && reset > observed
                    && reset <= next_observed
                    && next_reset - reset > 60_000)
                    .then_some(reset)
            })
            .collect();
        let mut index = 0;
        Ok((0..64)
            .map(|sample| {
                let timestamp = start + ((now - start) as f64 * sample as f64 / 63.) as i64;
                while index + 1 < points.len() && points[index + 1].0 <= timestamp {
                    index += 1;
                }
                let (observed, _, point) = points[index];
                let remaining = (timestamp <= cutoff).then(|| {
                    if resets
                        .iter()
                        .any(|reset| *reset > observed && *reset <= timestamp)
                    {
                        100.
                    } else {
                        (point.remaining_basis_points / 100.).clamp(0., 100.)
                    }
                });
                HistorySample {
                    timestamp,
                    remaining,
                }
            })
            .collect())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BalancePoint {
    pub observed_at: String,
    pub estimated_reset_at: String,
    pub remaining_basis_points: f64,
    pub plan_type: String,
}

#[derive(Clone, Debug, Default)]
pub struct Access {
    pub visible: bool,
    pub manage_credits: bool,
    pub app_limits: bool,
    pub view_credits: bool,
    pub analytics: bool,
    pub balance_history: bool,
    pub enterprise: bool,
    pub logged_in_pricing: bool,
    pub maximum_credits: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nullable_protocol_fields_do_not_invent_a_balance() {
        let limits: Limits = serde_json::from_value(json!({
            "rateLimits": {"primary": {"usedPercent": 101}},
            "rateLimitsByLimitId": null, "rateLimitResetCredits": null
        }))
        .unwrap();
        assert!(limits.rate_limits.credits.is_none());
        assert_eq!(limits.rate_limits.primary.unwrap().remaining(), 0.);
        let credits: ResetCredits = serde_json::from_value(json!({
            "availableCount": 2, "credits": null
        }))
        .unwrap();
        assert_eq!(credits.available_count, 2);
        assert!(credits.credits.is_empty());
    }

    #[test]
    fn reload_requires_whole_balances_and_preserves_the_grandfathered_target() {
        let settings = Reload::default();
        let mut draft = ReloadDraft {
            threshold: "125".into(),
            target: "250".into(),
            monthly: String::new(),
        };
        assert!(draft.validate(&settings, 250).is_ok());
        draft.threshold = "126".into();
        assert!(draft.validate(&settings, 250).is_err());
        draft.threshold = "125.0".into();
        assert!(draft.validate(&settings, 250).is_err());
        draft.threshold = "125".into();
        draft.monthly = "249".into();
        assert!(draft.validate(&settings, 250).is_err());
        draft.monthly.clear();
        draft.target = "300".into();
        assert!(draft.validate(&settings, 250).is_err());
        let previous = Reload {
            is_enabled: true,
            recharge_target: Some("300".into()),
            ..Default::default()
        };
        assert!(draft.validate(&previous, 250).is_ok());
        let previous = Reload {
            recharge_monthly_limit: Some("500".into()),
            ..previous
        };
        assert!(draft.payload(&previous)["recharge_monthly_limit"].is_null());
    }

    #[test]
    fn only_a_failed_top_up_warns_and_only_a_decline_offers_payment_recovery() {
        let warn = |status: &str, enabled: bool| {
            top_up_warning(&json!({"immediate_top_up_status": status, "is_enabled": enabled}))
        };
        assert_eq!(warn("succeeded", true), None);
        assert_eq!(warn("not_required", false), None);
        assert_eq!(top_up_warning(&json!({"is_enabled": true})), None);
        let (text, declined) = warn("payment_declined", false).unwrap();
        assert!(
            declined && text.starts_with("Auto-reload is off. The initial top-up was declined")
        );
        let (text, declined) = warn("failed", true).unwrap();
        assert!(!declined && text.starts_with("Auto-reload is on, but we couldn’t confirm"));
    }

    #[test]
    fn history_carries_observations_and_leaves_unreported_time_empty() {
        let history: BalanceHistory = serde_json::from_value(json!({
            "data_as_of": "2026-10-20T02:00:00Z",
            "points": [
                {"observed_at":"2026-10-20T00:00:00Z", "estimated_reset_at":"2026-10-20T01:00:00Z", "remaining_basis_points":2000, "plan_type":"pro"},
                {"observed_at":"2026-10-20T02:00:00Z", "estimated_reset_at":"2026-10-20T06:00:00Z", "remaining_basis_points":8000, "plan_type":"pro"}
            ]
        })).unwrap();
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-20T03:00:00Z")
            .unwrap()
            .timestamp_millis();
        let samples = history.samples(now).unwrap();
        assert_eq!(samples[0].remaining, Some(20.));
        assert_eq!(samples[21].remaining, Some(100.));
        assert_eq!(samples[42].remaining, Some(80.));
        assert_eq!(samples[43].remaining, None);
        assert_eq!(samples[63].timestamp, now);
    }
}
