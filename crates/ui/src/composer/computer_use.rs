use serde_json::{Value, json};

#[derive(Clone, Debug)]
pub struct ComputerUseApproval {
    pub app: String,
    pub display_name: String,
    pub connector_name: String,
    pub subtitle: Option<String>,
    pub high_risk: bool,
    pub allow_session: bool,
    pub allow_always: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum ComputerUseChoice {
    Once,
    Conversation,
    Always,
    Decline,
    Cancel,
}

impl ComputerUseApproval {
    pub fn response(&self, choice: ComputerUseChoice) -> Option<Value> {
        match choice {
            ComputerUseChoice::Once => Some(json!({"action": "accept", "content": {}})),
            ComputerUseChoice::Conversation if self.allow_session => Some(json!({
                "action": "accept", "content": {}, "_meta": {"persist": "session"}
            })),
            ComputerUseChoice::Always if self.allow_always => Some(json!({
                "action": "accept", "content": {}, "_meta": {"persist": "always"}
            })),
            ComputerUseChoice::Decline => Some(json!({"action": "decline"})),
            ComputerUseChoice::Cancel => Some(json!({"action": "cancel"})),
            _ => None,
        }
    }
}
