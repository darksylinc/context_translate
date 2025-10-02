use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::error;

pub struct AiSettings {
    pub endpoint: String,
    pub api_key: String,
    pub system_prompt: String,
    pub model: String,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message<'a>>,
}

#[derive(Serialize)]
struct Message<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Deserialize, Debug)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
struct Choice {
    message: MessageResponse,
}

#[derive(Deserialize, Debug)]
struct MessageResponse {
    role: String,
    content: String,
}

pub async fn run_prompt(
    ai_data: &AiSettings,
    prompt: String,
) -> Result<String, Box<dyn std::error::Error>> {
    println!("Running Prompt:\n{}", prompt);

    let client = reqwest::Client::new();

    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", ai_data.api_key))?,
    );
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    // Build request body
    let request_body = ChatRequest {
        model: &ai_data.model,
        messages: vec![
            Message {
                role: "system",
                content: &ai_data.system_prompt,
            },
            Message {
                role: "user",
                content: &prompt,
            },
        ],
    };

    // Send POST request
    let res = client
        .post(&ai_data.endpoint)
        .headers(headers)
        .json(&request_body)
        .send()
        .await?;

    if !res.status().is_success() {
        let status_code = res.status();
        eprintln!("Error: {}", res.text().await?);
        return Err(Box::new(error::Error::HttpStatus(status_code.as_u16())));
    }

    // Parse response
    let mut chat_response: ChatResponse = res.json().await?;
    for choice in &chat_response.choices {
        println!("AI Output:\n{}", choice.message.content);
    }

    let response = {
        if !chat_response.choices.is_empty() {
            let last_idx = chat_response.choices.len() - 1;
            chat_response.choices.swap(0, last_idx);
            chat_response.choices.pop().unwrap().message.content
        } else {
            String::new()
        }
    };

    Ok(response)
}
