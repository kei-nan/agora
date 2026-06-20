use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[derive(Serialize)]
struct ClaudeRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    system: &'a str,
    messages: Vec<ClaudeMessage>,
}

#[derive(Serialize)]
struct ClaudeMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ClaudeResponse {
    content: Vec<ClaudeContentBlock>,
}

#[derive(Deserialize)]
struct ClaudeContentBlock {
    text: String,
}

const SYSTEM_PROMPT: &str = "\
You are a civic AI assistant for a democratic blockchain governance platform. \
You help citizens understand laws, proposals, court rulings, and treasury transactions. \
Be concise, factual, and non-partisan. Cite the specific text provided in your context. \
If a question requires information beyond the provided context, say so rather than speculating. \
Never suggest how to vote or make political endorsements.";

/// Sends a question to Claude with the on-chain item as context.
/// Requires CLAUDE_API_KEY to be set in the environment.
/// Returns an error string (not Err) when offline so the frontend can degrade gracefully.
#[tauri::command]
pub async fn agent_ask(
    question: String,
    item_context: String,
    history: Vec<Message>,
) -> Result<String, String> {
    let api_key = std::env::var("CLAUDE_API_KEY")
        .map_err(|_| "CLAUDE_API_KEY not configured. Set it in your environment to enable AI features.".to_string())?;

    let client = reqwest::Client::new();

    // Build message history: prepend item context into the first user message
    let mut messages: Vec<ClaudeMessage> = Vec::new();
    for (i, msg) in history.iter().enumerate() {
        let content = if i == 0 && msg.role == "user" && !item_context.is_empty() {
            format!("Context:\n{item_context}\n\nQuestion: {}", msg.content)
        } else {
            msg.content.clone()
        };
        messages.push(ClaudeMessage { role: msg.role.clone(), content });
    }

    // Current question (with context if history is empty)
    let question_content = if history.is_empty() && !item_context.is_empty() {
        format!("Context:\n{item_context}\n\nQuestion: {question}")
    } else {
        question
    };
    messages.push(ClaudeMessage { role: "user".into(), content: question_content });

    let body = ClaudeRequest {
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        system: SYSTEM_PROMPT,
        messages,
    };

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_connect() || e.is_timeout() {
                "network".to_string()
            } else {
                e.to_string()
            }
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("API error {status}: {text}"));
    }

    let parsed: ClaudeResponse = resp.json().await.map_err(|e| e.to_string())?;
    parsed
        .content
        .into_iter()
        .next()
        .map(|b| b.text)
        .ok_or_else(|| "Empty response from AI".into())
}
