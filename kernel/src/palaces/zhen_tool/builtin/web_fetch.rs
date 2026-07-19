use crate::error::ToolError;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use crate::palaces::zhen_tool::base::BaseTool;
use crate::stems::action::ExecContext;
use crate::stems::intent::{CeremoniesIntent};

pub struct WebFetchTool {
    #[allow(dead_code)]
    client: reqwest::Client,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("jia/0.1.0")
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client builder");
        Self { client }
    }
}

#[async_trait]
impl BaseTool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> String {
        "Fetch content from a URL and convert HTML to plain text. \
         Returns the page text content. Use for reading web pages."
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
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, input: Value, _ctx: &ExecContext) -> Result<String, ToolError> {
        let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;

        let parsed = url::Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;

        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return Err(format!(
                "Unsupported URL scheme '{}': only http/https allowed",
                parsed.scheme()
            )
            .into());
        }

        // SSRF protection: resolve hostname and block private/reserved IPs
        if let Some(host) = parsed.host_str()
            && let Ok(addrs) = tokio::net::lookup_host(format!("{host}:0")).await
        {
            for addr in addrs {
                let blocked = match addr.ip() {
                    std::net::IpAddr::V4(v4) => {
                        v4.is_loopback()
                            || v4.is_private()
                            || v4.is_unspecified()
                            || v4.is_link_local()
                    }
                    std::net::IpAddr::V6(v6) => {
                        v6.is_loopback() || v6.is_unspecified() || v6.is_unique_local()
                    }
                };
                if blocked {
                    return Err(format!(
                        "SSRF blocked: URL resolves to private/reserved IP {}",
                        addr.ip()
                    )
                    .into());
                }
            }
        }

        let response = self
            .client
            .get(parsed.as_str())
            .send()
            .await
            .map_err(|e| format!("Request failed: {e}"))?;

        let status = response.status();
        let is_html = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|ct| {
                ct.to_string().contains("text/html") || ct.to_string().contains("application/xhtml")
            });

        // Check Content-Length header to avoid OOM on large responses
        const MAX_BODY_BYTES: usize = 10 * 1024 * 1024; // 10 MB
        if let Some(cl) = response.content_length() {
            if cl > MAX_BODY_BYTES as u64 {
                return Err(crate::error::ToolError::InvalidInput {
                    tool: "web_fetch".into(),
                    reason: format!(
                        "Response body too large: {} bytes (max {})",
                        cl, MAX_BODY_BYTES
                    ),
                });
            }
        }

        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {e}"))?;

        let text = if is_html { html_to_text(&body) } else { body };

        let max_len = 32000;
        let truncated = if text.chars().count() > max_len {
            format!(
                "{}... (truncated from {} chars)",
                crate::utils::truncate_chars(&text, max_len - 4), // -4 for "..."
                text.chars().count()
            )
        } else {
            text
        };

        Ok(format!(
            "URL: {}\nStatus: {}\n\n{}",
            parsed, status, truncated,
        ))
    }
}

/// Strip HTML tags, remove script/style blocks, decode common entities.
fn html_to_text(html: &str) -> String {
    let lower = html.to_lowercase();
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut skip_block = false;
    let mut chars = html.char_indices().peekable();

    while let Some((i, ch)) = chars.next() {
        if ch == '<' {
            let rest = &lower[i..];
            if rest.starts_with("<script") || rest.starts_with("<style") {
                skip_block = true;
                continue;
            }
            if rest.starts_with("</script") || rest.starts_with("</style") {
                skip_block = false;
                // Skip to '>'
                for (_, c) in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
                continue;
            }
            if !skip_block {
                in_tag = true;
            }
            continue;
        }

        if ch == '>' {
            in_tag = false;
            continue;
        }

        if in_tag || skip_block {
            continue;
        }

        text.push(ch);
    }

    // Single-pass: decode entities and collapse whitespace
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut last_was_newline = false;

    while let Some(ch) = chars.next() {
        if ch == '&' {
            // Collect entity
            let mut entity = String::with_capacity(16);
            entity.push('&');
            while let Some(&nc) = chars.peek() {
                if nc == ';' {
                    entity.push(';');
                    chars.next();
                    break;
                }
                if entity.len() > 12 || !nc.is_alphanumeric() && nc != '#' {
                    break;
                }
                entity.push(nc);
                chars.next();
            }
            match decode_entity(&entity) {
                Some(decoded) => {
                    result.push(decoded);
                }
                None => result.push_str(&entity),
            }
            last_was_newline = false;
        } else if ch == '\n' || ch == '\r' {
            if !last_was_newline {
                result.push('\n');
                last_was_newline = true;
            }
        } else if ch.is_whitespace() {
            if !last_was_newline && !result.ends_with(' ') {
                result.push(' ');
            }
        } else {
            result.push(ch);
            last_was_newline = false;
        }
    }

    // Trim empty lines
    result.trim().to_string()
}

fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "&amp;" => Some('&'),
        "&lt;" => Some('<'),
        "&gt;" => Some('>'),
        "&quot;" => Some('"'),
        "&#39;" | "&apos;" => Some('\''),
        "&nbsp;" => Some(' '),
        _ => {
            // Numeric entities: &#123; or &#x1F600;
            if entity.starts_with("&#") && entity.ends_with(';') {
                let inner = &entity[2..entity.len() - 1];
                if let Some(hex) = inner.strip_prefix('x') {
                    u32::from_str_radix(hex, 16)
                        .ok()
                        .and_then(std::char::from_u32)
                } else {
                    inner.parse::<u32>().ok().and_then(std::char::from_u32)
                }
            } else {
                None
            }
        }
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

    #[test]
    fn test_html_to_text_basic() {
        let html = "<html><body><h1>Title</h1><p>Hello <b>world</b></p></body></html>";
        let text = html_to_text(html);
        assert!(text.to_string().contains("Title"));
        assert!(text.to_string().contains("Hello world"));
    }

    #[test]
    fn test_html_to_text_removes_script() {
        let html = "<html><script>alert('xss')</script><p>Safe</p></html>";
        let text = html_to_text(html);
        assert!(!text.to_string().contains("alert"));
        assert!(text.to_string().contains("Safe"));
    }

    #[test]
    fn test_html_to_text_decodes_entities() {
        let html = "<p>Hello &amp; goodbye</p>";
        let text = html_to_text(html);
        assert!(text.to_string().contains("&"));
        assert!(!text.to_string().contains("&amp;"));
    }

    #[test]
    fn test_decode_numeric_entity() {
        assert_eq!(decode_entity("&#64;"), Some('@'));
        assert_eq!(decode_entity("&#x3E;"), Some('>'));
    }

    #[tokio::test]
    async fn web_fetch_missing_url() {
        let tool = WebFetchTool::new();
        let result = tool.execute(serde_json::json!({}), &test_ctx()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn web_fetch_invalid_scheme() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(
                serde_json::json!({"url": "file:///etc/passwd"}),
                &test_ctx(),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unsupported"));
    }
}
