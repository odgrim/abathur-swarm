//! Best-effort parser for human responses in ClickUp comments.

use abathur::domain::models::a2a::Artifact;
use regex::Regex;
use std::collections::HashSet;

use crate::clickup::models::ClickUpComment;

/// Parsed data extracted from human comments.
#[derive(Debug, Clone)]
pub struct ParsedResponse {
    pub summary: String,
    pub artifacts: Vec<Artifact>,
    /// Raw structured data extracted from JSON blocks in comments.
    /// Used by consumers needing access to the original parsed JSON.
    #[allow(dead_code)]
    pub structured_data: Option<serde_json::Value>,
}

/// Parse human response from ClickUp comments.
///
/// Comments are processed newest-first. Extracts JSON blocks, URLs,
/// and keyword lines to build a structured response.
pub fn parse_human_response(comments: &[ClickUpComment]) -> ParsedResponse {
    let mut sorted: Vec<&ClickUpComment> = comments.iter().collect();
    sorted.sort_by(|a, b| b.date.cmp(&a.date));

    let mut summary: Option<String> = None;
    let mut artifacts: Vec<Artifact> = Vec::new();
    let mut structured_data: Option<serde_json::Value> = None;
    let mut seen_values: HashSet<String> = HashSet::new();

    let json_re = Regex::new(r"(?s)```json\s*\n(.*?)\n\s*```").unwrap();
    let url_re = Regex::new(r#"https?://[^\s<>")\]]+"#).unwrap();
    let keyword_re = Regex::new(r"(?mi)^(Status|Result|Notes|Account|Reference|URL):\s*(.+)$").unwrap();

    for comment in &sorted {
        let text = &comment.comment_text;

        // 1. JSON block extraction
        for cap in json_re.captures_iter(text) {
            if let Some(json_str) = cap.get(1)
                && let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str.as_str())
                && structured_data.is_none()
            {
                // Extract summary from JSON if present
                if summary.is_none()
                    && let Some(s) = val.get("summary").and_then(|v| v.as_str())
                {
                    summary = Some(s.to_string());
                }
                // Extract artifacts from JSON if present
                if let Some(arts) = val.get("artifacts").and_then(|v| v.as_array()) {
                    for art in arts {
                        let art_type = art
                            .get("type")
                            .or_else(|| art.get("artifact_type"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("link");
                        let art_value = art
                            .get("value")
                            .or_else(|| art.get("url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or_default();
                        if !art_value.is_empty() && seen_values.insert(art_value.to_string()) {
                            artifacts.push(Artifact::new(art_type, art_value));
                        }
                    }
                }
                structured_data = Some(val);
            }
        }

        // 2. URL extraction
        for m in url_re.find_iter(text) {
            let url = m.as_str().to_string();
            if seen_values.insert(url.clone()) {
                let art_type = classify_url(&url);
                artifacts.push(Artifact::new(art_type, &url));
            }
        }

        // 3. Keyword line extraction
        for cap in keyword_re.captures_iter(text) {
            let key = cap.get(1).unwrap().as_str();
            let value = cap.get(2).unwrap().as_str().trim();
            if key.eq_ignore_ascii_case("Result") && summary.is_none() {
                summary = Some(value.to_string());
            }
        }
    }

    // 4. Fallback: use most recent comment text
    let summary = summary.unwrap_or_else(|| {
        sorted
            .first()
            .map(|c| {
                let text = &c.comment_text;
                if text.len() > 2000 {
                    format!("{}...", &text[..2000])
                } else {
                    text.clone()
                }
            })
            .unwrap_or_else(|| "No response provided".to_string())
    });

    ParsedResponse {
        summary,
        artifacts,
        structured_data,
    }
}

fn classify_url(url: &str) -> &'static str {
    if url.contains("github.com") && url.contains("/pull/") {
        "pr_url"
    } else if url.contains("docs.google.com") || url.ends_with(".pdf") || url.ends_with(".docx") {
        "doc_link"
    } else if url.contains("console.aws.com")
        || url.contains("console.cloud.google.com")
        || url.contains("portal.azure.com")
    {
        "cloud_console_url"
    } else {
        "link"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_comment(text: &str, date: &str) -> ClickUpComment {
        ClickUpComment {
            id: "1".to_string(),
            comment_text: text.to_string(),
            date: date.to_string(),
            user: crate::clickup::models::ClickUpUser {
                id: 1,
                username: "testuser".to_string(),
            },
        }
    }

    #[test]
    fn test_parse_json_block() {
        let comments = vec![make_comment(
            "Done!\n```json\n{\"summary\": \"Account created\", \"artifacts\": [{\"type\": \"link\", \"value\": \"https://example.com\"}]}\n```",
            "2024-01-02",
        )];
        let result = parse_human_response(&comments);
        assert_eq!(result.summary, "Account created");
        assert_eq!(result.artifacts.len(), 1);
        assert!(result.structured_data.is_some());
    }

    #[test]
    fn test_parse_urls() {
        let comments = vec![make_comment(
            "Here's the PR: https://github.com/org/repo/pull/42 and the console: https://console.aws.com/s3",
            "2024-01-02",
        )];
        let result = parse_human_response(&comments);
        assert_eq!(result.artifacts.len(), 2);
        assert_eq!(result.artifacts[0].artifact_type, "pr_url");
        assert_eq!(result.artifacts[1].artifact_type, "cloud_console_url");
    }

    #[test]
    fn test_parse_keyword_lines() {
        let comments = vec![make_comment(
            "Result: Bank account opened successfully\nAccount: 1234567890",
            "2024-01-02",
        )];
        let result = parse_human_response(&comments);
        assert_eq!(result.summary, "Bank account opened successfully");
    }

    #[test]
    fn test_fallback_to_comment_text() {
        let comments = vec![make_comment("Just finished the task", "2024-01-02")];
        let result = parse_human_response(&comments);
        assert_eq!(result.summary, "Just finished the task");
    }

    #[test]
    fn test_empty_comments() {
        let result = parse_human_response(&[]);
        assert_eq!(result.summary, "No response provided");
        assert!(result.artifacts.is_empty());
    }
}
