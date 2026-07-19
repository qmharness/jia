use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent};

pub struct WebSearchTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("jia/0.1.0")
            .build()
            .expect("reqwest client builder");
        Self { client }
    }
}

#[async_trait]
impl BaseTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> String {
        "Search the web using DuckDuckGo Instant Answer API. \
         Returns abstract summary and related topics. \
         No API key required."
            .to_string()
    }

    fn category(&self) -> &str {
        "web"
    }

    fn ceremony(&self) -> CeremoniesIntent {
        CeremoniesIntent::Ren
    }

    fn is_concurrency_safe(&self) -> bool {
        true
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let query = input["query"].as_str().ok_or("Missing 'query' parameter")?;

        if query.trim().is_empty() {
            return Err("Query cannot be empty".into());
        }

        let encoded = utf8_percent_encode(query, NON_ALPHANUMERIC);
        let url = format!(
            "https://api.duckduckgo.com/?q={encoded}&format=json&no_html=1&skip_disambig=1"
        );

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("Search request failed: {e}"))?;

        let status = response.status();
        let body: Value = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse response: {e}"))?;

        let mut output = String::new();
        output.push_str(&format!("Search: {query}\nStatus: {status}\n"));

        let abstract_text = body["AbstractText"].as_str().unwrap_or("");
        let abstract_source = body["AbstractSource"].as_str().unwrap_or("");
        let abstract_url = body["AbstractURL"].as_str().unwrap_or("");

        if !abstract_text.is_empty() {
            output.push_str(&format!("\n## Summary\n{abstract_text}\n"));
            if !abstract_source.is_empty() {
                output.push_str(&format!("Source: {abstract_source}"));
                if !abstract_url.is_empty() {
                    output.push_str(&format!(" ({abstract_url})"));
                }
                output.push('\n');
            }
        }

        let related = &body["RelatedTopics"];
        if let Some(topics) = related.as_array() {
            let mut count = 0;
            for topic in topics.iter().take(10) {
                if let Some(text) = topic["Text"].as_str() {
                    count += 1;
                    let url = topic["FirstURL"].as_str().unwrap_or("");
                    output.push_str(&format!("\n{count}. {text}"));
                    if !url.is_empty() {
                        output.push_str(&format!("\n   {url}"));
                    }
                    output.push('\n');
                }
            }
        }

        if let Some(infobox) = body["Infobox"].as_object()
            && let Some(content) = infobox["content"].as_array()
        {
            output.push_str("\n## Quick Facts\n");
            for item in content.iter().take(8) {
                let label = item["label"].as_str().unwrap_or("");
                let value = item["value"].as_str().unwrap_or("");
                if !label.is_empty() && !value.is_empty() {
                    output.push_str(&format!("- {label}: {value}\n"));
                }
            }
        }

        if abstract_text.is_empty() && related.as_array().is_none_or(|a| a.is_empty()) {
            output.push_str("\nNo results found.");
        }

        let max_len = 16000;
        if output.len() > max_len {
            output.truncate(max_len);
            output.push_str("... (truncated)");
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use crate::palaces::qian_permission::PermissionMatrix;
    use std::sync::Arc;
    fn test_ctx() -> crate::stems::action::ExecContext {
        use crate::palaces::qian_permission::PermissionMatrix;
        use std::sync::Arc;
        crate::stems::action::ExecContext::new(Arc::new(PermissionMatrix::default()))
    }

    use super::*;

    fn test_perms() -> Arc<PermissionMatrix> {
        Arc::new(PermissionMatrix::default())
    }

    #[tokio::test]
    async fn web_search_missing_query() {
        let tool = WebSearchTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn web_search_empty_query() {
        let tool = WebSearchTool::new();
        let result = tool
            .execute(serde_json::json!({"query": "   "}), &test_ctx())
            .await;
        assert!(result.is_err());
    }
}
