use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountProfile {
    pub id: String,
    pub provider: String, // "antigravity", "chatgpt", "claude"
    pub name: String,
    pub email: Option<String>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>, // "Pro", "Ultra", "Plus", "Team", "Free", etc.
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub is_active: bool,
    pub quota: Option<AccountQuota>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub name: String,
    pub used_percent: Option<f64>,
    pub remaining_percent: Option<f64>,
    pub resets_at: Option<String>,
    pub reset_in_seconds: Option<i64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountQuota {
    pub provider: String,
    pub account_id: String,
    pub email: Option<String>,
    pub plan_type: Option<String>,
    pub fetched_at: String,
    pub windows: Vec<QuotaWindow>,
    pub extra_info: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSwitchResult {
    pub success: bool,
    pub restarted_app: bool,
    pub message: String,
}
