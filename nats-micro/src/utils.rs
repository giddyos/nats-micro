use uuid::Uuid;

use crate::request::Headers;

pub fn ensure_request_id(headers: &Headers) -> String {
    if let Some(v) = headers.get("x-request-id") {
        return v.as_str().to_string();
    }
    if let Some(v) = headers.get("X-Request-Id") {
        return v.as_str().to_string();
    }
    Uuid::now_v7().to_string()
}

pub fn split_subject(subject: &str) -> Vec<&str> {
    subject.split('.').filter(|s| !s.is_empty()).collect()
}

pub fn extract_subject_param(subject_template: &str, subject: &str, name: &str) -> Option<String> {
    let template_parts = split_subject(subject_template);
    let subject_parts = split_subject(subject);

    if template_parts.len() != subject_parts.len() {
        return None;
    }

    for (template, actual) in template_parts.iter().zip(subject_parts.iter()) {
        if template.starts_with('{') && template.ends_with('}') {
            let key = &template[1..template.len() - 1];
            if key == name {
                return Some((*actual).to_string());
            }
        } else if *template != *actual {
            return None;
        }
    }

    None
}
