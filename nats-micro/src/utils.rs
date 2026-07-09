use uuid::Uuid;

use crate::request::Headers;

const MAX_REQUEST_ID_LEN: usize = 128;

fn is_valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_REQUEST_ID_LEN
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/' | b'@')
        })
}

pub fn ensure_request_id(headers: &Headers) -> String {
    if let Some(v) = headers
        .get("x-request-id")
        .filter(|header| is_valid_request_id(header.as_str()))
    {
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

#[cfg(test)]
mod tests {
    use super::ensure_request_id;
    use crate::request::Headers;

    #[test]
    fn ensure_request_id_accepts_bounded_safe_ascii_values() {
        let mut headers = Headers::new();
        headers.insert("x-request-id", "trace-123_abc.def:/@");

        assert_eq!(ensure_request_id(&headers), "trace-123_abc.def:/@");
    }

    #[test]
    fn ensure_request_id_replaces_empty_long_or_unsafe_values() {
        for value in [
            String::new(),
            "a".repeat(129),
            "bad\nid".to_string(),
            "snowman-☃".to_string(),
        ] {
            let mut headers = Headers::new();
            headers.insert("x-request-id", value.as_str());

            let request_id = ensure_request_id(&headers);

            assert_ne!(request_id, value);
            assert_eq!(request_id.len(), 36);
            assert!(
                request_id
                    .bytes()
                    .all(|byte| { byte.is_ascii_alphanumeric() || matches!(byte, b'-') })
            );
        }
    }
}
