use super::{
    internal_error, APIError, APIResponse, CompletionParams, Generation, Ide, RequestParams, NAME,
    VERSION,
};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Display;

use crate::error::{Error, Result};

fn build_tgi_body(prompt: String, params: &RequestParams) -> Value {
    serde_json::json!({
        "inputs": prompt,
        "parameters": {
            "max_new_tokens": params.max_new_tokens,
            "temperature": params.temperature,
            "do_sample": params.do_sample,
            "top_p": params.top_p,
            "stop_tokens": params.stop_tokens.clone()
        },
    })
}

fn build_tgi_headers(api_token: Option<&String>, ide: Ide) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let user_agent = format!("{NAME}/{VERSION}; rust/unknown; ide/{ide:?}");
    headers.insert(USER_AGENT, HeaderValue::from_str(&user_agent)?);

    if let Some(api_token) = api_token {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {api_token}"))?,
        );
    }

    Ok(headers)
}

fn parse_tgi_text(text: &str) -> Result<Vec<Generation>> {
    match serde_json::from_str(text)? {
        APIResponse::Generation(gen) => Ok(vec![gen]),
        APIResponse::Generations(_) => Err(Error::InvalidAdaptor),
        APIResponse::Error(err) => Err(Error::Tgi(err)),
    }
}

fn build_api_body(prompt: String, params: &RequestParams) -> Value {
    build_tgi_body(prompt, params)
}

fn build_api_headers(api_token: Option<&String>, ide: Ide) -> Result<HeaderMap> {
    build_tgi_headers(api_token, ide)
}

fn parse_api_text(text: &str) -> Result<Vec<Generation>> {
    match serde_json::from_str(text)? {
        APIResponse::Generation(gen) => Ok(vec![gen]),
        APIResponse::Generations(gens) => Ok(gens),
        APIResponse::Error(err) => Err(Error::InferenceApi(err)),
    }
}

fn build_ollama_body(prompt: String, params: &CompletionParams) -> Value {
    serde_json::json!({
        "prompt": prompt,
        "model": params.request_body.as_ref().ok_or_else(|| internal_error("missing request_body")).expect("Unable to make request for ollama").get("model"),
        "stream": false,
        // As per [modelfile](https://github.com/jmorganca/ollama/blob/main/docs/modelfile.md#valid-parameters-and-values)
        "options": {
            "num_predict": params.request_params.max_new_tokens,
            "temperature": params.request_params.temperature,
            "top_p": params.request_params.top_p,
            "stop": params.request_params.stop_tokens.clone(),
        }
    })
}
fn build_ollama_headers() -> Result<HeaderMap> {
    Ok(HeaderMap::new())
}

#[derive(Debug, Serialize, Deserialize)]
struct OllamaGeneration {
    response: String,
}

impl From<OllamaGeneration> for Generation {
    fn from(value: OllamaGeneration) -> Self {
        Generation {
            generated_text: value.response,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OllamaAPIResponse {
    Generation(OllamaGeneration),
    Error(APIError),
}

fn parse_ollama_text(text: &str) -> Result<Vec<Generation>> {
    match serde_json::from_str(text)? {
        OllamaAPIResponse::Generation(gen) => Ok(vec![gen.into()]),
        OllamaAPIResponse::Error(err) => Err(Error::Ollama(err)),
    }
}

fn build_openai_body(prompt: String, params: &CompletionParams) -> Value {
    serde_json::json!({
        "prompt": prompt,
        "model": params.request_body.as_ref().ok_or_else(|| internal_error("missing request_body")).expect("Unable to make request for openai").get("model"),
        "max_tokens": params.request_params.max_new_tokens,
        "temperature": params.request_params.temperature,
        "top_p": params.request_params.top_p,
        "stop": params.request_params.stop_tokens.clone(),
    })
}

fn build_openai_headers(api_token: Option<&String>, ide: Ide) -> Result<HeaderMap> {
    build_api_headers(api_token, ide)
}

#[derive(Debug, Deserialize)]
struct OpenAIGenerationChoice {
    text: String,
}

impl From<OpenAIGenerationChoice> for Generation {
    fn from(value: OpenAIGenerationChoice) -> Self {
        Generation {
            generated_text: value.text,
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAIGeneration {
    choices: Vec<OpenAIGenerationChoice>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAIErrorLoc {
    String(String),
    Int(u32),
}

impl Display for OpenAIErrorLoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenAIErrorLoc::String(s) => s.fmt(f),
            OpenAIErrorLoc::Int(i) => i.fmt(f),
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAIErrorDetail {
    loc: OpenAIErrorLoc,
    msg: String,
    r#type: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIError {
    detail: Vec<OpenAIErrorDetail>,
}

impl Display for OpenAIError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, item) in self.detail.iter().enumerate() {
            if i != 0 {
                writeln!(f)?;
            }
            write!(f, "{}: {} ({})", item.loc, item.msg, item.r#type)?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OpenAIAPIResponse {
    Generation(OpenAIGeneration),
    Error(OpenAIError),
}

fn parse_openai_text(text: &str) -> Result<Vec<Generation>> {
    let open_ai_response = serde_json::from_str(text)?;
    match open_ai_response {
        OpenAIAPIResponse::Generation(completion) => {
            Ok(completion.choices.into_iter().map(|x| x.into()).collect())
        }
        OpenAIAPIResponse::Error(err) => Err(Error::OpenAI(err)),
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Adaptor {
    #[default]
    HuggingFace,
    Ollama,
    OpenAi,
    Tgi,
}

impl Display for Adaptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::HuggingFace => write!(f, "huggingface"),
            Self::Ollama => write!(f, "ollama"),
            Self::OpenAi => write!(f, "openai"),
            Self::Tgi => write!(f, "tgi"),
        }
    }
}

pub fn adapt_body(prompt: String, params: &CompletionParams) -> Result<Value> {
    match params.adaptor.as_ref().unwrap_or(&Adaptor::default()) {
        Adaptor::HuggingFace => Ok(build_api_body(prompt, &params.request_params)),
        Adaptor::Ollama => Ok(build_ollama_body(prompt, params)),
        Adaptor::OpenAi => Ok(build_openai_body(prompt, params)),
        Adaptor::Tgi => Ok(build_tgi_body(prompt, &params.request_params)),
    }
}

pub fn adapt_headers(
    adaptor: Option<&Adaptor>,
    api_token: Option<&String>,
    ide: Ide,
) -> Result<HeaderMap> {
    match adaptor.unwrap_or(&Adaptor::default()) {
        Adaptor::HuggingFace => build_api_headers(api_token, ide),
        Adaptor::Ollama => build_ollama_headers(),
        Adaptor::OpenAi => build_openai_headers(api_token, ide),
        Adaptor::Tgi => build_tgi_headers(api_token, ide),
    }
}

pub fn parse_generations(adaptor: Option<&Adaptor>, text: &str) -> Result<Vec<Generation>> {
    match adaptor.unwrap_or(&Adaptor::default()) {
        Adaptor::HuggingFace => parse_api_text(text),
        Adaptor::Ollama => parse_ollama_text(text),
        Adaptor::OpenAi => parse_openai_text(text),
        Adaptor::Tgi => parse_tgi_text(text),
    }
}
