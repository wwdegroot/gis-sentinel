use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AlertType {
    Update,
    New,
    Remove,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SentinelAlert{
    pub id: String,
    pub name: String,
    pub atype: AlertType,
    pub performance: i32,
    pub expected: i32,
    pub up: bool,
    pub reason: String,
    pub error: Option<String>
}