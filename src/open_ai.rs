use std::time::Duration;

use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};

use crate::error;

pub struct AiSettings<'a> {
    pub endpoint: String,
    pub api_key: String,
    pub system_prompt: String,
    pub model: String,
    pub timeout_secs: u64,
    pub extra_options: Option<&'a serde_json::Map<String, serde_json::Value>>,
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
    ai_data: &AiSettings<'_>,
    prompt: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    // println!("Running Prompt:\n{}", prompt);

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

    let request_body = {
        match ai_data.extra_options {
            Some(extra_opts) => {
                let mut merged = serde_json::to_value(&request_body).unwrap();
                let obj = merged.as_object_mut().unwrap();
                for (k, v) in extra_opts {
                    obj.insert(k.clone(), v.clone());
                }
                merged
            }
            None => serde_json::to_value(&request_body).unwrap(),
        }
    };

    // println!("JSON:\n{}", request_body);

    // Send POST request
    let res = client
        .post(&ai_data.endpoint)
        .headers(headers)
        .json(&request_body)
        .send();

    let res = match tokio::time::timeout(Duration::from_secs(ai_data.timeout_secs), res).await {
        Ok(res) => res?,
        Err(_) => {
            eprintln!("AI took too long. Aborting.");
            return Ok("".to_string());
        }
    };

    if !res.status().is_success() {
        let status_code = res.status();
        eprintln!("Error: {}", res.text().await?);
        return Err(Box::new(error::Error::HttpStatus(status_code.as_u16())));
    }

    // Parse response
    let mut chat_response: ChatResponse = res.json().await?;
    /*for choice in &chat_response.choices {
        println!("AI Output:\n{}", choice.message.content);
    }*/

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
