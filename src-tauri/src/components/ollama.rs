use crate::domain::{
    operation_explanation, validate_edit_plan, EditOperation, EditPlan, ImageQualityAnalysis,
    OllamaModel, PlanValidationReport,
};
use crate::error::AppError;
use reqwest::{Client, Method, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

pub const OLLAMA_PROVIDER_VERSION: &str = "1";
pub const MAX_OLLAMA_PROMPT_CHARS: usize = 1_000;
const MAX_WARNINGS: usize = 8;
const MAX_TEXT_CHARS: usize = 240;

#[derive(Debug, Clone)]
pub struct OllamaClient {
    client: Client,
    endpoint: String,
    maximum_response_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OllamaGeneration {
    pub model: String,
    pub response: String,
}

#[derive(Debug)]
pub struct PlanValidationFailure {
    pub error: AppError,
    pub report: Box<PlanValidationReport>,
}

impl OllamaClient {
    pub fn new(
        endpoint: &str,
        timeout_ms: u64,
        maximum_response_bytes: u64,
    ) -> Result<Self, AppError> {
        let endpoint = validate_ollama_endpoint(endpoint)?;
        let timeout = Duration::from_millis(timeout_ms);
        let client = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| AppError::InvalidPlannerEndpoint)?;
        Ok(Self {
            client,
            endpoint,
            maximum_response_bytes,
        })
    }

    pub async fn version(&self) -> Result<String, AppError> {
        let bytes = self.request(Method::GET, "/api/version", None).await?;
        let parsed: VersionResponse = parse_json(&bytes)?;
        if parsed.version.trim().is_empty()
            || parsed
                .version
                .split('.')
                .next()
                .and_then(|part| part.parse::<u32>().ok())
                .is_none()
        {
            return Err(AppError::UnsupportedPlannerVersion);
        }
        Ok(parsed.version)
    }

    pub async fn models(&self) -> Result<Vec<OllamaModel>, AppError> {
        let bytes = self.request(Method::GET, "/api/tags", None).await?;
        let parsed: TagsResponse = parse_json(&bytes)?;
        Ok(parsed
            .models
            .into_iter()
            .map(|model| {
                let mut capabilities = BTreeSet::new();
                capabilities.extend(model.capabilities);
                if let Some(details) = model.details {
                    if !details.format.trim().is_empty() {
                        capabilities.insert(format!("format: {}", details.format));
                    }
                    if !details.family.trim().is_empty() {
                        capabilities.insert(format!("family: {}", details.family));
                    }
                    capabilities.extend(
                        details
                            .families
                            .into_iter()
                            .flatten()
                            .filter(|value| !value.trim().is_empty())
                            .map(|value| format!("family: {value}")),
                    );
                }
                OllamaModel {
                    name: model.name,
                    size_bytes: model.size,
                    modified_at: model.modified_at,
                    capabilities: capabilities.into_iter().collect(),
                }
            })
            .collect())
    }

    pub async fn generate(
        &self,
        model: &str,
        request: &str,
        analysis: &ImageQualityAnalysis,
        maximum_operations: usize,
    ) -> Result<OllamaGeneration, AppError> {
        if model.trim().is_empty() {
            return Err(AppError::OllamaModelMissing);
        }
        let prompt = deterministic_planner_prompt(request, analysis, maximum_operations)?;
        let body = json!({
            "model": model,
            "prompt": prompt,
            "stream": false,
            "format": ollama_plan_schema(maximum_operations),
            "options": {
                "temperature": 0,
                "seed": 0
            }
        });
        let bytes = self
            .request(Method::POST, "/api/generate", Some(body))
            .await?;
        let parsed: GenerateResponse = parse_json(&bytes)?;
        if !parsed.done || parsed.model.trim().is_empty() || parsed.response.trim().is_empty() {
            return Err(AppError::OllamaJsonParse);
        }
        Ok(OllamaGeneration {
            model: parsed.model,
            response: parsed.response,
        })
    }

    async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Vec<u8>, AppError> {
        let url = format!("{}{}", self.endpoint, path);
        let mut request = self
            .client
            .request(method, url)
            .header("accept", "application/json");
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        if !status.is_success() {
            return if status.as_u16() == 404 {
                Err(AppError::OllamaModelMissing)
            } else {
                Err(AppError::OllamaHttpStatus(status.as_u16()))
            };
        }
        if response
            .content_length()
            .is_some_and(|length| length > self.maximum_response_bytes)
        {
            return Err(AppError::OllamaResponseTooLarge {
                limit: self.maximum_response_bytes,
            });
        }
        let capacity = usize::try_from(self.maximum_response_bytes.min(64 * 1_024)).unwrap_or(0);
        let mut bytes = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(map_reqwest_error)? {
            let next_length = bytes.len().saturating_add(chunk.len());
            if u64::try_from(next_length).unwrap_or(u64::MAX) > self.maximum_response_bytes {
                return Err(AppError::OllamaResponseTooLarge {
                    limit: self.maximum_response_bytes,
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

pub fn validate_ollama_endpoint(endpoint: &str) -> Result<String, AppError> {
    let parsed = Url::parse(endpoint.trim()).map_err(|_| AppError::InvalidPlannerEndpoint)?;
    if parsed.scheme() != "http"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !matches!(parsed.path(), "" | "/")
        || !matches!(
            parsed.host_str(),
            Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
        )
        || parsed.port_or_known_default().is_none()
    {
        return Err(AppError::InvalidPlannerEndpoint);
    }
    Ok(endpoint.trim().trim_end_matches('/').to_string())
}

pub fn deterministic_planner_prompt(
    request: &str,
    analysis: &ImageQualityAnalysis,
    maximum_operations: usize,
) -> Result<String, AppError> {
    let request = request.trim();
    if request.is_empty() || request.chars().count() > MAX_OLLAMA_PROMPT_CHARS {
        return Err(AppError::OllamaSchemaValidation(format!(
            "user requests must contain 1 to {MAX_OLLAMA_PROMPT_CHARS} characters"
        )));
    }
    if maximum_operations == 0 || maximum_operations > 8 {
        return Err(AppError::OllamaSchemaValidation(
            "maximum generated operations must be between one and eight".into(),
        ));
    }
    let payload = json!({
        "userRequest": request,
        "imageAnalysisSummary": analysis,
        "supportedOperations": supported_operations(),
        "parameterRanges": parameter_ranges(),
        "jsonSchema": ollama_plan_schema(maximum_operations)
    });
    serde_json::to_string(&payload).map_err(|_| AppError::OllamaJsonParse)
}

pub fn validate_ollama_plan(
    raw_response: &str,
    maximum_operations: usize,
) -> Result<(EditPlan, PlanValidationReport), PlanValidationFailure> {
    let started = Instant::now();
    let mut report = PlanValidationReport {
        original_response: raw_response.to_string(),
        ..PlanValidationReport::default()
    };
    if raw_response.len() > 2_097_152 {
        report
            .errors
            .push("response exceeds the hard 2 MiB validation limit".into());
        report.validation_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
        return Err(validation_failure(
            AppError::OllamaResponseTooLarge { limit: 2_097_152 },
            report,
        ));
    }
    if maximum_operations == 0 || maximum_operations > 8 {
        report
            .errors
            .push("maximum generated operations must be between one and eight".into());
        report.validation_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
        return Err(validation_failure(
            AppError::OllamaSchemaValidation(report.errors[0].clone()),
            report,
        ));
    }
    let value: Value = match serde_json::from_str(raw_response) {
        Ok(value) => value,
        Err(_) => {
            report.errors.push("response is not valid JSON".into());
            report.validation_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
            return Err(validation_failure(AppError::OllamaJsonParse, report));
        }
    };
    inspect_unknown_fields(&value, &mut report);
    if !report.rejected_fields.is_empty() {
        report
            .errors
            .push("unknown fields are not allowed in planner responses".into());
    }
    let wire: OllamaPlanWire = match serde_json::from_value(value) {
        Ok(wire) => wire,
        Err(error) => {
            report.errors.push(sanitize_serde_error(&error.to_string()));
            report.validation_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
            return Err(validation_failure(
                AppError::OllamaSchemaValidation(report.errors.join("; ")),
                report,
            ));
        }
    };
    if wire.operations.len() > maximum_operations {
        report.errors.push(format!(
            "the response contains {} operations; the configured maximum is {maximum_operations}",
            wire.operations.len()
        ));
    }
    if wire.warnings.len() > MAX_WARNINGS {
        report
            .errors
            .push(format!("at most {MAX_WARNINGS} warnings are allowed"));
    }
    if wire.summary.trim().is_empty() || wire.summary.chars().count() > MAX_TEXT_CHARS {
        report.errors.push(format!(
            "summary must contain between 1 and {MAX_TEXT_CHARS} characters"
        ));
    }
    if wire
        .warnings
        .iter()
        .any(|warning| warning.trim().is_empty() || warning.chars().count() > MAX_TEXT_CHARS)
    {
        report.errors.push(format!(
            "warnings must contain between 1 and {MAX_TEXT_CHARS} characters"
        ));
    }
    if !wire.confidence.is_finite() || !(0.0..=1.0).contains(&wire.confidence) {
        report
            .errors
            .push("confidence must be a finite number between 0 and 1".into());
    }
    let operations = wire
        .operations
        .into_iter()
        .map(EditOperation::from)
        .collect::<Vec<_>>();
    let plan = EditPlan {
        summary: wire.summary,
        confidence: wire.confidence,
        warnings: wire.warnings,
        operation_explanations: operations
            .iter()
            .map(|operation| operation_explanation(operation).to_string())
            .collect(),
        operations,
    };
    if let Err(error) = validate_edit_plan(&plan) {
        report.errors.push(error.to_string());
    }
    if !report.errors.is_empty() {
        report.validation_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
        return Err(validation_failure(
            AppError::OllamaSchemaValidation(report.errors.join("; ")),
            report,
        ));
    }
    report.valid = true;
    report.validated_response = serde_json::to_string_pretty(&plan).ok();
    report.validation_time_ms = started.elapsed().as_secs_f64() * 1_000.0;
    Ok((plan, report))
}

fn validation_failure(error: AppError, report: PlanValidationReport) -> PlanValidationFailure {
    PlanValidationFailure {
        error,
        report: Box::new(report),
    }
}

fn parse_json<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, AppError> {
    let text = std::str::from_utf8(bytes).map_err(|_| AppError::OllamaJsonParse)?;
    serde_json::from_str(text).map_err(|_| AppError::OllamaJsonParse)
}

fn map_reqwest_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        return AppError::OllamaTimeout;
    }
    let message = error.to_string().to_ascii_lowercase();
    if error.is_connect() && message.contains("refused") {
        AppError::OllamaConnectionRefused
    } else {
        AppError::OllamaHostUnreachable
    }
}

fn inspect_unknown_fields(value: &Value, report: &mut PlanValidationReport) {
    let Some(root) = value.as_object() else {
        return;
    };
    let expected = ["summary", "confidence", "warnings", "operations"];
    for field in root.keys().filter(|key| !expected.contains(&key.as_str())) {
        report.rejected_fields.push(field.clone());
    }
    let Some(operations) = root.get("operations").and_then(Value::as_array) else {
        return;
    };
    for (index, operation) in operations.iter().enumerate() {
        let Some(object) = operation.as_object() else {
            continue;
        };
        let Some(kind) = object.get("type").and_then(Value::as_str) else {
            continue;
        };
        let allowed = allowed_operation_fields(kind);
        for field in object.keys().filter(|key| !allowed.contains(&key.as_str())) {
            report
                .rejected_fields
                .push(format!("operations[{index}].{field}"));
        }
    }
}

fn allowed_operation_fields(kind: &str) -> &'static [&'static str] {
    match kind {
        "brightness" | "contrast" | "saturation" => &["type", "amount"],
        "grayscale" => &["type"],
        "auto_white_balance" | "deblock" => &["type", "strength"],
        "local_contrast" => &["type", "strength", "tile_size", "clip_limit"],
        "denoise" => &["type", "strength", "preserve_edges"],
        "edge_aware_sharpen" => &["type", "strength", "radius", "threshold"],
        "mild_deblur" | "uneven_lighting_correction" => &["type", "strength", "radius"],
        "document_enhance" => &["type", "strength", "grayscale"],
        _ => &["type"],
    }
}

fn sanitize_serde_error(error: &str) -> String {
    error
        .split(" at line ")
        .next()
        .unwrap_or("response does not match the required schema")
        .chars()
        .take(240)
        .collect()
}

fn supported_operations() -> Vec<&'static str> {
    vec![
        "brightness",
        "contrast",
        "saturation",
        "grayscale",
        "auto_white_balance",
        "local_contrast",
        "denoise",
        "deblock",
        "edge_aware_sharpen",
        "mild_deblur",
        "document_enhance",
        "uneven_lighting_correction",
    ]
}

fn parameter_ranges() -> Value {
    json!({
        "brightness.amount": [-1.0, 1.0],
        "contrast.amount": [-1.0, 1.0],
        "saturation.amount": [-1.0, 1.0],
        "auto_white_balance.strength": [0.0, 1.0],
        "local_contrast.strength": [0.0, 1.0],
        "local_contrast.tile_size": [8, 128],
        "local_contrast.clip_limit": [0.5, 4.0],
        "denoise.strength": [0.0, 1.0],
        "denoise.preserve_edges": [0.0, 1.0],
        "deblock.strength": [0.0, 1.0],
        "edge_aware_sharpen.strength": [0.0, 2.0],
        "edge_aware_sharpen.radius": [0.5, 4.0],
        "edge_aware_sharpen.threshold": [0.0, 0.25],
        "mild_deblur.strength": [0.0, 1.0],
        "mild_deblur.radius": [0.5, 3.0],
        "document_enhance.strength": [0.0, 1.0],
        "uneven_lighting_correction.strength": [0.0, 1.0],
        "uneven_lighting_correction.radius": [4.0, 96.0]
    })
}

fn ollama_plan_schema(maximum_operations: usize) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "confidence", "warnings", "operations"],
        "properties": {
            "summary": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARS},
            "confidence": {"type": "number", "minimum": 0, "maximum": 1},
            "warnings": {
                "type": "array",
                "maxItems": MAX_WARNINGS,
                "items": {"type": "string", "minLength": 1, "maxLength": MAX_TEXT_CHARS}
            },
            "operations": {
                "type": "array",
                "minItems": 1,
                "maxItems": maximum_operations,
                "items": {"oneOf": operation_schemas()}
            }
        }
    })
}

fn operation_schemas() -> Vec<Value> {
    vec![
        operation_schema(
            "brightness",
            json!({"amount": number_range(-1.0, 1.0)}),
            &["amount"],
        ),
        operation_schema(
            "contrast",
            json!({"amount": number_range(-1.0, 1.0)}),
            &["amount"],
        ),
        operation_schema(
            "saturation",
            json!({"amount": number_range(-1.0, 1.0)}),
            &["amount"],
        ),
        operation_schema("grayscale", json!({}), &[]),
        operation_schema(
            "auto_white_balance",
            json!({"strength": number_range(0.0, 1.0)}),
            &["strength"],
        ),
        operation_schema(
            "local_contrast",
            json!({
                "strength": number_range(0.0, 1.0),
                "tile_size": {"type": "integer", "minimum": 8, "maximum": 128},
                "clip_limit": number_range(0.5, 4.0)
            }),
            &["strength", "tile_size", "clip_limit"],
        ),
        operation_schema(
            "denoise",
            json!({
                "strength": number_range(0.0, 1.0),
                "preserve_edges": number_range(0.0, 1.0)
            }),
            &["strength", "preserve_edges"],
        ),
        operation_schema(
            "deblock",
            json!({"strength": number_range(0.0, 1.0)}),
            &["strength"],
        ),
        operation_schema(
            "edge_aware_sharpen",
            json!({
                "strength": number_range(0.0, 2.0),
                "radius": number_range(0.5, 4.0),
                "threshold": number_range(0.0, 0.25)
            }),
            &["strength", "radius", "threshold"],
        ),
        operation_schema(
            "mild_deblur",
            json!({
                "strength": number_range(0.0, 1.0),
                "radius": number_range(0.5, 3.0)
            }),
            &["strength", "radius"],
        ),
        operation_schema(
            "document_enhance",
            json!({
                "strength": number_range(0.0, 1.0),
                "grayscale": {"type": "boolean"}
            }),
            &["strength", "grayscale"],
        ),
        operation_schema(
            "uneven_lighting_correction",
            json!({
                "strength": number_range(0.0, 1.0),
                "radius": number_range(4.0, 96.0)
            }),
            &["strength", "radius"],
        ),
    ]
}

fn operation_schema(kind: &str, parameters: Value, required: &[&str]) -> Value {
    let mut properties = parameters.as_object().cloned().unwrap_or_default();
    properties.insert("type".into(), json!({"const": kind}));
    let mut required_fields = vec!["type"];
    required_fields.extend_from_slice(required);
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required_fields,
        "properties": properties
    })
}

fn number_range(minimum: f64, maximum: f64) -> Value {
    json!({"type": "number", "minimum": minimum, "maximum": maximum})
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    version: String,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<TagModel>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct TagModel {
    name: String,
    #[serde(default)]
    model: String,
    modified_at: String,
    size: u64,
    #[serde(default)]
    digest: String,
    #[serde(default)]
    details: Option<TagDetails>,
    #[serde(default)]
    capabilities: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)]
struct TagDetails {
    #[serde(default)]
    parent_model: String,
    #[serde(default)]
    format: String,
    #[serde(default)]
    family: String,
    #[serde(default)]
    families: Option<Vec<String>>,
    #[serde(default)]
    parameter_size: String,
    #[serde(default)]
    quantization_level: String,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GenerateResponse {
    model: String,
    #[serde(default)]
    created_at: String,
    response: String,
    #[serde(default)]
    thinking: String,
    done: bool,
    #[serde(default)]
    done_reason: String,
    #[serde(default)]
    context: Vec<u64>,
    #[serde(default)]
    total_duration: u64,
    #[serde(default)]
    load_duration: u64,
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    prompt_eval_duration: u64,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    eval_duration: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OllamaPlanWire {
    summary: String,
    confidence: f32,
    warnings: Vec<String>,
    operations: Vec<OllamaOperationWire>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum OllamaOperationWire {
    Brightness {
        amount: f32,
    },
    Contrast {
        amount: f32,
    },
    Saturation {
        amount: f32,
    },
    Grayscale,
    AutoWhiteBalance {
        strength: f32,
    },
    LocalContrast {
        strength: f32,
        tile_size: u32,
        clip_limit: f32,
    },
    Denoise {
        strength: f32,
        preserve_edges: f32,
    },
    Deblock {
        strength: f32,
    },
    EdgeAwareSharpen {
        strength: f32,
        radius: f32,
        threshold: f32,
    },
    MildDeblur {
        strength: f32,
        radius: f32,
    },
    DocumentEnhance {
        strength: f32,
        grayscale: bool,
    },
    UnevenLightingCorrection {
        strength: f32,
        radius: f32,
    },
}

impl From<OllamaOperationWire> for EditOperation {
    fn from(value: OllamaOperationWire) -> Self {
        match value {
            OllamaOperationWire::Brightness { amount } => Self::Brightness { amount },
            OllamaOperationWire::Contrast { amount } => Self::Contrast { amount },
            OllamaOperationWire::Saturation { amount } => Self::Saturation { amount },
            OllamaOperationWire::Grayscale => Self::Grayscale,
            OllamaOperationWire::AutoWhiteBalance { strength } => {
                Self::AutoWhiteBalance { strength }
            }
            OllamaOperationWire::LocalContrast {
                strength,
                tile_size,
                clip_limit,
            } => Self::LocalContrast {
                strength,
                tile_size,
                clip_limit,
            },
            OllamaOperationWire::Denoise {
                strength,
                preserve_edges,
            } => Self::Denoise {
                strength,
                preserve_edges,
            },
            OllamaOperationWire::Deblock { strength } => Self::Deblock { strength },
            OllamaOperationWire::EdgeAwareSharpen {
                strength,
                radius,
                threshold,
            } => Self::EdgeAwareSharpen {
                strength,
                radius,
                threshold,
            },
            OllamaOperationWire::MildDeblur { strength, radius } => {
                Self::MildDeblur { strength, radius }
            }
            OllamaOperationWire::DocumentEnhance {
                strength,
                grayscale,
            } => Self::DocumentEnhance {
                strength,
                grayscale,
            },
            OllamaOperationWire::UnevenLightingCorrection { strength, radius } => {
                Self::UnevenLightingCorrection { strength, radius }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ColorCastEstimate, EditPlanner, RulePlanner};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;

    fn analysis() -> ImageQualityAnalysis {
        ImageQualityAnalysis {
            average_luminance: 0.42,
            luminance_spread: 0.37,
            estimated_color_cast: ColorCastEstimate {
                dominant: "warm".into(),
                red_bias: 0.08,
                green_bias: -0.01,
                blue_bias: -0.07,
            },
            estimated_noise: 0.14,
            estimated_sharpness: 0.09,
            estimated_local_contrast: 0.06,
            edge_density: 0.11,
            white_background_ratio: 0.02,
            likely_document: false,
        }
    }

    fn valid_response(operation: &str) -> String {
        format!(
            r#"{{"summary":"A conservative local plan.","confidence":0.82,"warnings":[],"operations":[{operation}]}}"#
        )
    }

    struct MockServer {
        endpoint: String,
        request: Arc<Mutex<Vec<u8>>>,
    }

    impl MockServer {
        fn respond(status: u16, body: Vec<u8>, delay_ms: u64, content_length: bool) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let address = listener.local_addr().unwrap();
            let request = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&request);
            thread::spawn(move || {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
                read_request(&mut stream, &captured);
                if delay_ms > 0 {
                    thread::sleep(Duration::from_millis(delay_ms));
                }
                let reason = match status {
                    200 => "OK",
                    302 => "Found",
                    404 => "Not Found",
                    _ => "Error",
                };
                let length = if content_length {
                    format!("Content-Length: {}\r\n", body.len())
                } else {
                    "Connection: close\r\n".into()
                };
                let headers = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n{length}\r\n"
                );
                let _ = stream.write_all(headers.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            });
            Self {
                endpoint: format!("http://{address}"),
                request,
            }
        }
    }

    fn read_request(stream: &mut TcpStream, captured: &Arc<Mutex<Vec<u8>>>) {
        let mut buffer = [0_u8; 8 * 1_024];
        let mut request = Vec::new();
        while let Ok(count) = stream.read(&mut buffer) {
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
        }
        *captured.lock().unwrap() = request;
    }

    macro_rules! endpoint_accepts {
        ($name:ident, $value:literal) => {
            #[test]
            fn $name() {
                assert!(validate_ollama_endpoint($value).is_ok());
            }
        };
    }

    macro_rules! endpoint_rejects {
        ($name:ident, $value:literal) => {
            #[test]
            fn $name() {
                assert!(matches!(
                    validate_ollama_endpoint($value),
                    Err(AppError::InvalidPlannerEndpoint)
                ));
            }
        };
    }

    endpoint_accepts!(accepts_ipv4_default_port, "http://127.0.0.1:11434");
    endpoint_accepts!(accepts_localhost, "http://localhost:11434");
    endpoint_accepts!(accepts_ipv6_loopback, "http://[::1]:11434");
    endpoint_accepts!(accepts_loopback_without_port, "http://127.0.0.1");
    endpoint_accepts!(trims_trailing_slash, "http://127.0.0.1:11434/");
    endpoint_rejects!(rejects_https, "https://127.0.0.1:11434");
    endpoint_rejects!(rejects_remote_ipv4, "http://192.168.1.10:11434");
    endpoint_rejects!(rejects_remote_hostname, "http://ollama.example:11434");
    endpoint_rejects!(rejects_zero_host_trick, "http://127.0.0.1.example:11434");
    endpoint_rejects!(rejects_user_info, "http://user@127.0.0.1:11434");
    endpoint_rejects!(rejects_password, "http://user:pass@127.0.0.1:11434");
    endpoint_rejects!(rejects_path, "http://127.0.0.1:11434/api");
    endpoint_rejects!(rejects_query, "http://127.0.0.1:11434?x=1");
    endpoint_rejects!(rejects_fragment, "http://127.0.0.1:11434#x");
    endpoint_rejects!(rejects_empty_endpoint, "");
    endpoint_rejects!(rejects_file_scheme, "file:///tmp/ollama");

    macro_rules! accepts_operation {
        ($name:ident, $json:literal, $variant:pat) => {
            #[test]
            fn $name() {
                let (plan, report) = validate_ollama_plan(&valid_response($json), 8).unwrap();
                assert!(report.valid);
                assert!(matches!(plan.operations[0], $variant));
            }
        };
    }

    accepts_operation!(
        accepts_brightness,
        r#"{"type":"brightness","amount":0.1}"#,
        EditOperation::Brightness { .. }
    );
    accepts_operation!(
        accepts_contrast,
        r#"{"type":"contrast","amount":0.1}"#,
        EditOperation::Contrast { .. }
    );
    accepts_operation!(
        accepts_saturation,
        r#"{"type":"saturation","amount":0.1}"#,
        EditOperation::Saturation { .. }
    );
    accepts_operation!(
        accepts_grayscale,
        r#"{"type":"grayscale"}"#,
        EditOperation::Grayscale
    );
    accepts_operation!(
        accepts_white_balance,
        r#"{"type":"auto_white_balance","strength":0.5}"#,
        EditOperation::AutoWhiteBalance { .. }
    );
    accepts_operation!(
        accepts_local_contrast,
        r#"{"type":"local_contrast","strength":0.4,"tile_size":32,"clip_limit":1.2}"#,
        EditOperation::LocalContrast { .. }
    );
    accepts_operation!(
        accepts_denoise,
        r#"{"type":"denoise","strength":0.3,"preserve_edges":0.8}"#,
        EditOperation::Denoise { .. }
    );
    accepts_operation!(
        accepts_deblock,
        r#"{"type":"deblock","strength":0.3}"#,
        EditOperation::Deblock { .. }
    );
    accepts_operation!(
        accepts_edge_sharpen,
        r#"{"type":"edge_aware_sharpen","strength":0.3,"radius":1.0,"threshold":0.04}"#,
        EditOperation::EdgeAwareSharpen { .. }
    );
    accepts_operation!(
        accepts_mild_deblur,
        r#"{"type":"mild_deblur","strength":0.3,"radius":1.0}"#,
        EditOperation::MildDeblur { .. }
    );
    accepts_operation!(
        accepts_document_enhance,
        r#"{"type":"document_enhance","strength":0.7,"grayscale":false}"#,
        EditOperation::DocumentEnhance { .. }
    );
    accepts_operation!(
        accepts_uneven_lighting,
        r#"{"type":"uneven_lighting_correction","strength":0.5,"radius":32.0}"#,
        EditOperation::UnevenLightingCorrection { .. }
    );

    fn assert_rejected(raw: &str) -> PlanValidationReport {
        let failure = validate_ollama_plan(raw, 8).unwrap_err();
        assert!(!failure.report.valid);
        assert!(!failure.report.errors.is_empty());
        *failure.report
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(
            validate_ollama_plan("{", 8).unwrap_err().error,
            AppError::OllamaJsonParse
        ));
    }

    #[test]
    fn rejects_top_level_array() {
        assert_rejected("[]");
    }

    #[test]
    fn rejects_unknown_top_level_field_and_reports_it() {
        let report = assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"grayscale"}],"shell":"run"}"#,
        );
        assert_eq!(report.rejected_fields, ["shell"]);
    }

    #[test]
    fn rejects_unknown_operation_field_and_reports_path() {
        let report = assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"brightness","amount":0.1,"path":"secret"}]}"#,
        );
        assert_eq!(report.rejected_fields, ["operations[0].path"]);
    }

    #[test]
    fn rejects_missing_summary() {
        assert_rejected(r#"{"confidence":0.5,"warnings":[],"operations":[{"type":"grayscale"}]}"#);
    }

    #[test]
    fn rejects_missing_confidence() {
        assert_rejected(r#"{"summary":"x","warnings":[],"operations":[{"type":"grayscale"}]}"#);
    }

    #[test]
    fn rejects_missing_warnings() {
        assert_rejected(r#"{"summary":"x","confidence":0.5,"operations":[{"type":"grayscale"}]}"#);
    }

    #[test]
    fn rejects_missing_operations() {
        assert_rejected(r#"{"summary":"x","confidence":0.5,"warnings":[]}"#);
    }

    #[test]
    fn rejects_wrong_summary_type() {
        assert_rejected(
            r#"{"summary":4,"confidence":0.5,"warnings":[],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_wrong_confidence_type() {
        assert_rejected(
            r#"{"summary":"x","confidence":"high","warnings":[],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_wrong_warning_type() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[4],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_wrong_operations_type() {
        assert_rejected(r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":{}}"#);
    }

    #[test]
    fn rejects_unknown_operation() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"invent_pixels"}]}"#,
        );
    }

    #[test]
    fn rejects_missing_operation_parameter() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"brightness"}]}"#,
        );
    }

    #[test]
    fn rejects_invalid_parameter_name() {
        let report = assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"brightness","strength":0.1}]}"#,
        );
        assert_eq!(report.rejected_fields, ["operations[0].strength"]);
    }

    #[test]
    fn rejects_out_of_range_parameter() {
        assert_rejected(&valid_response(r#"{"type":"brightness","amount":2}"#));
    }

    #[test]
    fn rejects_empty_operations() {
        assert_rejected(r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[]}"#);
    }

    #[test]
    fn rejects_configured_operation_limit() {
        let raw = r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"brightness","amount":0.1},{"type":"contrast","amount":0.1}]}"#;
        assert!(validate_ollama_plan(raw, 1).is_err());
    }

    #[test]
    fn rejects_invalid_configured_operation_limit() {
        assert!(validate_ollama_plan(&valid_response(r#"{"type":"grayscale"}"#), 0).is_err());
    }

    #[test]
    fn rejects_confidence_above_one() {
        assert_rejected(
            r#"{"summary":"x","confidence":1.1,"warnings":[],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_negative_confidence() {
        assert_rejected(
            r#"{"summary":"x","confidence":-0.1,"warnings":[],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_empty_summary() {
        assert_rejected(
            r#"{"summary":" ","confidence":0.5,"warnings":[],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_long_summary() {
        let raw = json!({"summary":"x".repeat(241),"confidence":0.5,"warnings":[],"operations":[{"type":"grayscale"}]}).to_string();
        assert_rejected(&raw);
    }

    #[test]
    fn rejects_too_many_warnings() {
        let raw = json!({"summary":"x","confidence":0.5,"warnings":vec!["x"; 9],"operations":[{"type":"grayscale"}]}).to_string();
        assert_rejected(&raw);
    }

    #[test]
    fn rejects_empty_warning() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[""],"operations":[{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_long_warning() {
        let raw = json!({"summary":"x","confidence":0.5,"warnings":["x".repeat(241)],"operations":[{"type":"grayscale"}]}).to_string();
        assert_rejected(&raw);
    }

    #[test]
    fn rejects_duplicate_operations() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"brightness","amount":0.1},{"type":"brightness","amount":0.2}]}"#,
        );
    }

    #[test]
    fn rejects_conflicting_grayscale_and_saturation() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"saturation","amount":0.2},{"type":"grayscale"}]}"#,
        );
    }

    #[test]
    fn rejects_unsafe_operation_order() {
        assert_rejected(
            r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"edge_aware_sharpen","strength":0.2,"radius":1.0,"threshold":0.04},{"type":"denoise","strength":0.2,"preserve_edges":0.8}]}"#,
        );
    }

    #[test]
    fn adds_local_operation_explanations() {
        let (plan, _) =
            validate_ollama_plan(&valid_response(r#"{"type":"deblock","strength":0.3}"#), 8)
                .unwrap();
        assert_eq!(plan.operation_explanations.len(), 1);
        assert!(plan.operation_explanations[0].contains("8×8"));
    }

    #[test]
    fn report_preserves_original_response() {
        let raw = valid_response(r#"{"type":"grayscale"}"#);
        let (_, report) = validate_ollama_plan(&raw, 8).unwrap();
        assert_eq!(report.original_response, raw);
    }

    #[test]
    fn report_contains_read_only_validated_json() {
        let (_, report) =
            validate_ollama_plan(&valid_response(r#"{"type":"grayscale"}"#), 8).unwrap();
        assert!(report
            .validated_response
            .unwrap()
            .contains("operationExplanations"));
    }

    #[test]
    fn prompt_is_deterministic() {
        assert_eq!(
            deterministic_planner_prompt("reduce noise", &analysis(), 8).unwrap(),
            deterministic_planner_prompt("reduce noise", &analysis(), 8).unwrap()
        );
    }

    #[test]
    fn prompt_contains_only_approved_sections() {
        let prompt = deterministic_planner_prompt("reduce noise", &analysis(), 8).unwrap();
        let value: Value = serde_json::from_str(&prompt).unwrap();
        let keys = value
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(keys.len(), 5);
        assert!(value.get("userRequest").is_some());
        assert!(value.get("imageAnalysisSummary").is_some());
        assert!(value.get("supportedOperations").is_some());
        assert!(value.get("parameterRanges").is_some());
        assert!(value.get("jsonSchema").is_some());
    }

    #[test]
    fn prompt_contains_no_application_paths_or_os_data() {
        let prompt = deterministic_planner_prompt("reduce noise", &analysis(), 8).unwrap();
        for forbidden in [
            "filename",
            "filePath",
            "username",
            "LOCALAPPDATA",
            "Windows",
            "environment",
        ] {
            assert!(!prompt.contains(forbidden));
        }
    }

    #[test]
    fn prompt_contains_every_analysis_metric() {
        let prompt = deterministic_planner_prompt("reduce noise", &analysis(), 8).unwrap();
        for metric in [
            "averageLuminance",
            "luminanceSpread",
            "estimatedNoise",
            "estimatedSharpness",
            "estimatedLocalContrast",
            "edgeDensity",
            "whiteBackgroundRatio",
            "likelyDocument",
        ] {
            assert!(prompt.contains(metric));
        }
    }

    #[test]
    fn prompt_rejects_empty_request() {
        assert!(deterministic_planner_prompt(" ", &analysis(), 8).is_err());
    }

    #[test]
    fn prompt_rejects_large_request() {
        assert!(deterministic_planner_prompt(&"x".repeat(1001), &analysis(), 8).is_err());
    }

    #[test]
    fn prompt_schema_uses_configured_operation_limit() {
        let prompt = deterministic_planner_prompt("reduce noise", &analysis(), 3).unwrap();
        assert!(prompt.contains("\"maxItems\":3"));
    }

    #[test]
    fn prompt_schema_forbids_additional_properties() {
        let prompt = deterministic_planner_prompt("reduce noise", &analysis(), 8).unwrap();
        assert!(prompt.contains("\"additionalProperties\":false"));
    }

    #[test]
    fn creating_client_does_not_connect() {
        let client = OllamaClient::new("http://127.0.0.1:9", 250, 1024).unwrap();
        assert_eq!(client.endpoint, "http://127.0.0.1:9");
    }

    #[test]
    fn mock_server_returns_version() {
        let server = MockServer::respond(200, br#"{"version":"0.11.0"}"#.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        let version = tauri::async_runtime::block_on(client.version()).unwrap();
        assert_eq!(version, "0.11.0");
    }

    #[test]
    fn mock_server_rejects_unsupported_version() {
        let server = MockServer::respond(200, br#"{"version":"unknown"}"#.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::UnsupportedPlannerVersion)
        ));
    }

    #[test]
    fn mock_server_discovers_empty_model_list() {
        let server = MockServer::respond(200, br#"{"models":[]}"#.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(tauri::async_runtime::block_on(client.models())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn mock_server_discovers_model_metadata() {
        let body = br#"{"models":[{"name":"gemma3:4b","model":"gemma3:4b","modified_at":"2026-01-01T00:00:00Z","size":123,"digest":"abc","details":{"parent_model":"","format":"gguf","family":"gemma3","families":["gemma3"],"parameter_size":"4B","quantization_level":"Q4"},"capabilities":["completion"]}]}"#;
        let server = MockServer::respond(200, body.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 8_192).unwrap();
        let models = tauri::async_runtime::block_on(client.models()).unwrap();
        assert_eq!(models[0].name, "gemma3:4b");
        assert!(models[0].capabilities.contains(&"completion".into()));
        assert!(models[0].capabilities.contains(&"format: gguf".into()));
    }

    #[test]
    fn mock_server_accepts_newer_optional_model_metadata() {
        let body = br#"{"models":[{"name":"qwen3.6:latest","model":"qwen3.6:latest","modified_at":"2026-07-13T13:40:35Z","size":23938333577,"digest":"abc","details":{"parent_model":"","format":"gguf","family":"qwen35moe","families":["qwen35moe"],"parameter_size":"36.0B","quantization_level":"Q4_K_M","context_length":262144,"embedding_length":2048},"capabilities":["vision","completion","tools","thinking"],"future_metadata":{"safe_to_ignore":true}}],"future_page_metadata":1}"#;
        let server = MockServer::respond(200, body.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 8_192).unwrap();
        let models = tauri::async_runtime::block_on(client.models()).unwrap();
        assert_eq!(models[0].name, "qwen3.6:latest");
        assert!(models[0].capabilities.contains(&"thinking".into()));
        assert!(models[0].capabilities.contains(&"family: qwen35moe".into()));
    }

    #[test]
    fn mock_server_handles_malformed_json() {
        let server = MockServer::respond(200, b"{".to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaJsonParse)
        ));
    }

    #[test]
    fn mock_server_handles_invalid_utf8() {
        let server = MockServer::respond(200, vec![0xff, 0xfe], 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaJsonParse)
        ));
    }

    #[test]
    fn mock_server_handles_http_error() {
        let server = MockServer::respond(500, br#"{"error":"failed"}"#.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaHttpStatus(500))
        ));
    }

    #[test]
    fn mock_server_maps_not_found_to_model_missing() {
        let server = MockServer::respond(404, br#"{"error":"not found"}"#.to_vec(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaModelMissing)
        ));
    }

    #[test]
    fn mock_server_enforces_content_length_limit() {
        let server = MockServer::respond(200, vec![b'x'; 2_000], 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 1_024).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaResponseTooLarge { .. })
        ));
    }

    #[test]
    fn mock_server_enforces_streamed_size_limit() {
        let server = MockServer::respond(200, vec![b'x'; 2_000], 0, false);
        let client = OllamaClient::new(&server.endpoint, 1_000, 1_024).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaResponseTooLarge { .. })
        ));
    }

    #[test]
    fn mock_server_handles_timeout_without_retry() {
        let server = MockServer::respond(200, br#"{"version":"0.11.0"}"#.to_vec(), 350, true);
        let client = OllamaClient::new(&server.endpoint, 100, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaTimeout)
        ));
    }

    #[test]
    fn mock_server_does_not_follow_redirects() {
        let server = MockServer::respond(302, Vec::new(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.version()),
            Err(AppError::OllamaHttpStatus(302))
        ));
    }

    #[test]
    fn connection_refused_is_typed() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let client = OllamaClient::new(&format!("http://{address}"), 250, 4_096).unwrap();
        let error = tauri::async_runtime::block_on(client.version()).unwrap_err();
        assert!(matches!(
            error,
            AppError::OllamaConnectionRefused
                | AppError::OllamaHostUnreachable
                | AppError::OllamaTimeout
        ));
    }

    #[test]
    fn mock_server_generates_plan_json() {
        let inner = valid_response(r#"{"type":"grayscale"}"#);
        let body = json!({"model":"gemma3:4b","created_at":"now","response":inner,"done":true})
            .to_string();
        let server = MockServer::respond(200, body.into_bytes(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 64 * 1_024).unwrap();
        let generated = tauri::async_runtime::block_on(client.generate(
            "gemma3:4b",
            "grayscale",
            &analysis(),
            8,
        ))
        .unwrap();
        assert_eq!(generated.model, "gemma3:4b");
        assert!(generated.response.contains("grayscale"));
    }

    #[test]
    fn phase_five_performance_sample() {
        let connection_server =
            MockServer::respond(200, br#"{"version":"0.11.0"}"#.to_vec(), 0, true);
        let connection_client =
            OllamaClient::new(&connection_server.endpoint, 1_000, 64 * 1_024).unwrap();
        let connection_started = Instant::now();
        tauri::async_runtime::block_on(connection_client.version()).unwrap();
        let connection_ms = connection_started.elapsed().as_secs_f64() * 1_000.0;

        let inner = valid_response(r#"{"type":"grayscale"}"#);
        let body = json!({"model":"gemma3:4b","response":inner,"done":true}).to_string();
        let generation_server = MockServer::respond(200, body.into_bytes(), 0, true);
        let generation_client =
            OllamaClient::new(&generation_server.endpoint, 1_000, 64 * 1_024).unwrap();
        let comparison_started = Instant::now();
        let rule_started = Instant::now();
        RulePlanner
            .create_plan("reduce noise and sharpen slightly", &analysis())
            .unwrap();
        let rule_ms = rule_started.elapsed().as_secs_f64() * 1_000.0;
        let generation_started = Instant::now();
        let generated = tauri::async_runtime::block_on(generation_client.generate(
            "gemma3:4b",
            "reduce noise and sharpen slightly",
            &analysis(),
            8,
        ))
        .unwrap();
        let generation_ms = generation_started.elapsed().as_secs_f64() * 1_000.0;
        let (_, report) = validate_ollama_plan(&generated.response, 8).unwrap();
        let comparison_ms = comparison_started.elapsed().as_secs_f64() * 1_000.0;

        let large_prompt_started = Instant::now();
        assert!(deterministic_planner_prompt(&"x".repeat(1_001), &analysis(), 8).is_err());
        let large_prompt_ms = large_prompt_started.elapsed().as_secs_f64() * 1_000.0;
        eprintln!(
            "PHASE5_PERF connection_ms={connection_ms:.3} generation_ms={generation_ms:.3} validation_ms={:.3} rule_ms={rule_ms:.3} comparison_ms={comparison_ms:.3} large_prompt_rejection_ms={large_prompt_ms:.3}",
            report.validation_time_ms
        );

        assert!(connection_ms < 1_000.0);
        assert!(generation_ms < 1_000.0);
        assert!(comparison_ms < 1_000.0);
        assert!(!report.original_response.is_empty());
    }

    #[test]
    fn generate_request_disables_streaming_and_randomness() {
        let inner = valid_response(r#"{"type":"grayscale"}"#);
        let body = json!({"model":"gemma3:4b","response":inner,"done":true}).to_string();
        let server = MockServer::respond(200, body.into_bytes(), 0, true);
        let client = OllamaClient::new(&server.endpoint, 1_000, 64 * 1_024).unwrap();
        tauri::async_runtime::block_on(client.generate("gemma3:4b", "grayscale", &analysis(), 8))
            .unwrap();
        let request = String::from_utf8_lossy(&server.request.lock().unwrap()).to_string();
        assert!(request.contains("\"stream\":false"));
        assert!(request.contains("\"temperature\":0"));
        assert!(request.contains("\"seed\":0"));
        assert!(!request.contains("C:\\\\"));
    }

    #[test]
    fn generate_requires_selected_model() {
        let client = OllamaClient::new("http://127.0.0.1:9", 250, 4_096).unwrap();
        assert!(matches!(
            tauri::async_runtime::block_on(client.generate("", "grayscale", &analysis(), 8)),
            Err(AppError::OllamaModelMissing)
        ));
    }
}
