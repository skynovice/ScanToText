use std::{collections::HashMap, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::blocking::Client;
use serde::{
    Deserialize, Deserializer,
    de::{Error as DeError, MapAccess, Visitor},
};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{
    config::{Config, validate_key},
    image_ops::BatchRow,
};

const OCRSPACE_URL: &str = "https://api.ocr.space/parse/image";
const GEMINI_URL_PREFIX: &str = "https://generativelanguage.googleapis.com/v1beta/models/";

#[derive(Debug, Error)]
pub enum OcrError {
    #[error("{0}")]
    InvalidKey(String),
    #[error("HTTP: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Response(String),
}

fn client() -> Result<Client, reqwest::Error> {
    Client::builder().timeout(Duration::from_secs(60)).build()
}

fn gemini_text(value: &Value) -> Result<String, OcrError> {
    let candidate = value
        .get("candidates")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .ok_or_else(|| {
            OcrError::Response(format!(
                "Gemini returned no text: {:?}",
                value.get("promptFeedback")
            ))
        })?;
    if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str)
        && reason != "STOP"
        && candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        return Err(OcrError::Response(format!("Gemini stopped: {reason}")));
    }
    Ok(candidate
        .pointer("/content/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|part| {
            !part
                .get("thought")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect())
}

fn normal_roi_id(value: &str) -> String {
    let compact: String = value
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '_' | '-'))
        .collect();
    let upper = compact.to_ascii_uppercase();
    upper
        .strip_prefix("ROI")
        .and_then(|digits| digits.parse::<usize>().ok())
        .map(|number| format!("ROI_{number:03}"))
        .unwrap_or_else(|| value.to_owned())
}

pub fn parse_gemini_batch(value: &Value, expected: &[String]) -> Result<Vec<String>, OcrError> {
    let raw = gemini_text(value)?;
    if raw.trim().is_empty() {
        return Ok(vec![String::new(); expected.len()]);
    }
    let start = raw
        .find('{')
        .ok_or_else(|| OcrError::Response("Gemini ไม่ได้คืน JSON".into()))?;
    let end = raw
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or_else(|| OcrError::Response("Gemini ไม่ได้คืน JSON".into()))?;
    struct UniqueObject(HashMap<String, Value>);
    impl<'de> Deserialize<'de> for UniqueObject {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct UniqueVisitor;
            impl<'de> Visitor<'de> for UniqueVisitor {
                type Value = UniqueObject;
                fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("a JSON object with unique ROI fields")
                }
                fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                    let mut result = HashMap::new();
                    while let Some((key, value)) = map.next_entry::<String, Value>()? {
                        let normalized = normal_roi_id(&key);
                        if result.insert(normalized.clone(), value).is_some() {
                            return Err(A::Error::custom(format!("Duplicate field {normalized}")));
                        }
                    }
                    Ok(UniqueObject(result))
                }
            }
            deserializer.deserialize_map(UniqueVisitor)
        }
    }
    let parsed: UniqueObject = serde_json::from_str(&raw[start..=end])
        .map_err(|error| OcrError::Response(format!("Gemini JSON ไม่ถูกต้อง: {error}")))?;
    let normalized = parsed.0;
    if let Some(unknown) = normalized.keys().find(|key| !expected.contains(key)) {
        return Err(OcrError::Response(format!(
            "Gemini คืน field ที่ไม่รู้จัก: {unknown}"
        )));
    }
    expected
        .iter()
        .map(|id| {
            normalized
                .get(id)
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .pipe(Ok)
        })
        .collect()
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}

fn ocrspace_error(value: &Value) -> Result<(), OcrError> {
    if value
        .get("IsErroredOnProcessing")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let error = match value.get("ErrorMessage") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            Some(other) => other.to_string(),
            None => "OCR.space error".into(),
        };
        return Err(OcrError::Response(error));
    }
    Ok(())
}

fn normalize_ocr_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ponytail: merge only order-preserving supersets; unlabeled multi-ROI recovery needs per-ROI OCR.
fn is_strict_ocr_extension(candidate: &str, current: &str) -> bool {
    let candidate: String = candidate.chars().filter(|ch| !ch.is_whitespace()).collect();
    let current: String = current.chars().filter(|ch| !ch.is_whitespace()).collect();
    candidate.chars().count() > current.chars().count()
        && (current.is_empty() || candidate.contains(&current))
}

fn parsed_text_rows(value: &Value, rows: &[BatchRow]) -> Vec<String> {
    let joined = value
        .get("ParsedResults")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|parsed| parsed.get("ParsedText").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    if rows.len() == 1 {
        let row = &rows[0];
        let candidates = [row.id.clone(), row.id.replace('_', ""), "ROI_1".into()];
        let text = candidates
            .iter()
            .find_map(|marker| joined.split_once(marker).map(|(_, tail)| tail))
            .unwrap_or(&joined);
        let end = text.find("ROI").unwrap_or(text.len());
        return vec![normalize_ocr_text(&text[..end])];
    }

    let mut result = vec![String::new(); rows.len()];
    for (index, row) in rows.iter().enumerate() {
        let candidates = [
            row.id.clone(),
            row.id.replace('_', ""),
            format!("ROI_{}", index + 1),
        ];
        if let Some((_, tail)) = candidates
            .iter()
            .find_map(|marker| joined.split_once(marker))
        {
            let end = tail.find("ROI").unwrap_or(tail.len());
            result[index] = normalize_ocr_text(&tail[..end]);
        }
    }
    result
}

pub fn parse_ocrspace_batch(value: &Value, rows: &[BatchRow]) -> Result<Vec<String>, OcrError> {
    ocrspace_error(value)?;
    let mut result = vec![Vec::<String>::new(); rows.len()];
    let mut positioned_rows = vec![false; rows.len()];
    if let Some(parsed_results) = value.get("ParsedResults").and_then(Value::as_array) {
        for parsed in parsed_results {
            if let Some(lines) = parsed
                .pointer("/TextOverlay/Lines")
                .and_then(Value::as_array)
            {
                for line in lines {
                    let position = line
                        .get("MinTop")
                        .and_then(Value::as_f64)
                        .map(|top| {
                            (
                                top,
                                line.get("MaxHeight").and_then(Value::as_f64).unwrap_or(1.0),
                            )
                        })
                        .or_else(|| {
                            let words = line.get("Words")?.as_array()?;
                            let top = words
                                .iter()
                                .filter_map(|word| word.get("Top").and_then(Value::as_f64))
                                .reduce(f64::min)?;
                            let bottom = words
                                .iter()
                                .filter_map(|word| {
                                    Some(
                                        word.get("Top")?.as_f64()?
                                            + word
                                                .get("Height")
                                                .and_then(Value::as_f64)
                                                .unwrap_or(1.0),
                                    )
                                })
                                .reduce(f64::max)?;
                            Some((top, (bottom - top).max(1.0)))
                        });
                    let Some((top, height)) = position else {
                        continue;
                    };
                    let center = top + height / 2.0;
                    let line_text = line.get("LineText").and_then(Value::as_str).unwrap_or("");
                    let word_text;
                    let text = if line_text.trim().is_empty() {
                        word_text = line
                            .get("Words")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|word| word.get("WordText").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join(" ");
                        word_text.trim()
                    } else {
                        line_text.trim()
                    };
                    if text.is_empty() {
                        continue;
                    }
                    if let Some((index, _)) = rows
                        .iter()
                        .enumerate()
                        .find(|(_, row)| center >= row.top as f64 && center < row.bottom as f64)
                    {
                        result[index].push(text.to_owned());
                        positioned_rows[index] = true;
                    }
                }
            }
        }
    }
    let fallback = parsed_text_rows(value, rows);
    if positioned_rows.iter().any(|positioned| *positioned) {
        return Ok(result
            .into_iter()
            .enumerate()
            .map(|(index, parts)| {
                let overlay = normalize_ocr_text(&parts.join(" "));
                if is_strict_ocr_extension(&fallback[index], &overlay) {
                    fallback[index].clone()
                } else {
                    overlay
                }
            })
            .collect());
    }
    Ok(fallback)
}

fn post_ocrspace(jpeg: &[u8], config: &Config) -> Result<Value, OcrError> {
    validate_key("OCR.space", &config.ocrspace_key).map_err(OcrError::InvalidKey)?;
    let response = client()?
        .post(OCRSPACE_URL)
        .header("apikey", &config.ocrspace_key)
        .form(&[
            (
                "base64Image",
                format!("data:image/jpeg;base64,{}", STANDARD.encode(jpeg)),
            ),
            ("language", "auto".into()),
            ("OCREngine", "3".into()),
            ("scale", "true".into()),
            ("isOverlayRequired", "true".into()),
        ])
        .send()?;
    let status = response.status();
    let value: Value = response.json()?;
    if !status.is_success() {
        return Err(OcrError::Response(format!("HTTP {}", status.as_u16())));
    }
    Ok(value)
}

fn post_gemini(config: &Config, body: Value) -> Result<Value, OcrError> {
    validate_key("Gemini", &config.gemini_key).map_err(OcrError::InvalidKey)?;
    let model = config.gemini_model.trim();
    let url = format!("{GEMINI_URL_PREFIX}{model}:generateContent");
    let response = client()?
        .post(url)
        .header("x-goog-api-key", &config.gemini_key)
        .json(&body)
        .send()?;
    let status = response.status();
    let value: Value = response.json()?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("Gemini HTTP error");
        return Err(OcrError::Response(format!(
            "HTTP {}: {message}",
            status.as_u16()
        )));
    }
    Ok(value)
}

pub fn ocr_batch(jpeg: &[u8], config: &Config, rows: &[BatchRow]) -> Result<Vec<String>, OcrError> {
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    if config.engine == "gemini" {
        let encoded = STANDARD.encode(jpeg);
        let template = ids
            .iter()
            .map(|id| format!("\"{id}\":\"\""))
            .collect::<Vec<_>>()
            .join(", ");
        let prompt = format!(
            "This image contains multiple accounting-document crops. Gray ROI_### headers are metadata, not content. \
             Transcribe each crop exactly (digits, punctuation, Thai or English). Return ONLY one JSON object with exactly these keys: {{{template}}}. \
             Use an empty string when a crop has no text."
        );
        let body = json!({
            "contents": [{"parts": [
                {"text": prompt},
                {"inline_data": {"mime_type": "image/jpeg", "data": encoded}}
            ]}],
            "generationConfig": {"responseMimeType": "application/json", "temperature": 0}
        });
        return parse_gemini_batch(&post_gemini(config, body)?, &ids);
    }
    parse_ocrspace_batch(&post_ocrspace(jpeg, config)?, rows)
}

pub fn ocr_single_image(
    jpeg: &[u8],
    config: &Config,
    row: &BatchRow,
) -> Result<String, OcrError> {
    if config.engine == "gemini" {
        let body = json!({
            "contents": [{"parts": [
                {"text": "Transcribe all visible text in this image exactly. Preserve the original reading order. Return only the recognized text, without Markdown or explanation."},
                {"inline_data": {"mime_type": "image/jpeg", "data": STANDARD.encode(jpeg)}}
            ]}],
            "generationConfig": {"temperature": 0}
        });
        return Ok(normalize_ocr_text(&gemini_text(&post_gemini(config, body)?)?));
    }

    let value = post_ocrspace(jpeg, config)?;
    Ok(parse_ocrspace_batch(&value, std::slice::from_ref(row))?
        .into_iter()
        .next()
        .unwrap_or_default())
}

pub fn translate_image(jpeg: &[u8], config: &Config) -> Result<(String, String), OcrError> {
    let body = json!({
        "contents": [{"parts": [
            {"text": "Read all visible text in this image exactly, then translate it to Thai. Return ONLY valid JSON with exactly two string fields: {\"text\":\"original text\", \"thai_translation\":\"Thai translation\"}. Do not add Markdown or explanation. Preserve numbers, dates, and punctuation."},
            {"inline_data": {"mime_type": "image/jpeg", "data": STANDARD.encode(jpeg)}}
        ]}],
        "generationConfig": {"responseMimeType": "application/json", "temperature": 0}
    });
    let raw = gemini_text(&post_gemini(config, body)?)?;
    let start = raw
        .find('{')
        .ok_or_else(|| OcrError::Response("Gemini ไม่ได้คืน translation JSON".into()))?;
    let end = raw
        .rfind('}')
        .ok_or_else(|| OcrError::Response("Gemini ไม่ได้คืน translation JSON".into()))?;
    let parsed: Value = serde_json::from_str(&raw[start..=end])
        .map_err(|error| OcrError::Response(error.to_string()))?;
    let text = parsed
        .get("text")
        .or_else(|| parsed.get("source_text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let thai = parsed
        .get("thai_translation")
        .or_else(|| parsed.get("translation"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok((text.trim().into(), thai.trim().into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_gemini_and_overlay_rows() {
        let ids = vec!["ROI_001".into(), "ROI_002".into()];
        let gemini = json!({"candidates":[{"content":{"parts":[{"text":"```json\n{\"ROI_1\":\"1,234.50\",\"ROI_002\":\" บริษัท ก \"}\n```"}]}}]});
        assert_eq!(
            parse_gemini_batch(&gemini, &ids).unwrap(),
            ["1,234.50", "บริษัท ก"]
        );
        let rows = vec![
            BatchRow {
                id: ids[0].clone(),
                top: 40,
                bottom: 80,
            },
            BatchRow {
                id: ids[1].clone(),
                top: 100,
                bottom: 140,
            },
        ];
        let overlay = json!({"ParsedResults":[{"TextOverlay":{"Lines":[
            {"LineText":"TOTAL 100","MinTop":45,"MaxHeight":10},
            {"LineText":"INV-42","MinTop":105,"MaxHeight":10}
        ]}}]});
        assert_eq!(
            parse_ocrspace_batch(&overlay, &rows).unwrap(),
            ["TOTAL 100", "INV-42"]
        );
    }
}
