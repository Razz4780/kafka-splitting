use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub mod configuration;
pub mod crd;
pub mod k8s_util;
pub mod kafka;

#[derive(Deserialize, Serialize, Debug)]
pub struct ApiMessage {
    pub topic: String,
    pub partition: Option<i32>,
    pub payload: Option<String>,
    pub key: Option<String>,
    pub headers: Option<HashMap<String, Option<String>>>,
    pub timestamp: Option<i64>,
}
