use std::future::Future;

/// A generated compile-time service definition.
pub trait StaticService<S>: Send + Sync + 'static {
    const SPEC: crate::ServiceSpec;
}

/// Generated local routing over the same concrete endpoint dispatch functions.
pub trait LocalService<S>: StaticService<S> {
    fn dispatch_local<'a>(
        state: &'a S,
        operation: usize,
        request: crate::Request<'a>,
    ) -> impl Future<Output = crate::DispatchResult> + Send + 'a;
}

/// Static metadata for a generated operation marker.
#[derive(Debug, Clone, Copy)]
pub struct OperationMarker {
    pub index: usize,
    pub spec: &'static crate::OperationSpec,
}

/// A generated publish-only operation.
pub trait PublishOperation: Send + Sync + 'static {
    const SPEC: crate::OperationSpec;
}

#[must_use]
pub fn build_subject(prefix: Option<&str>, version: &str, group: &str, subject: &str) -> String {
    let prefix = prefix.filter(|value| !value.is_empty());
    let major = version
        .split('.')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or("0");

    let mut full_subject = String::with_capacity(
        prefix.map_or(0, |prefix| prefix.len() + 1)
            + 1
            + major.len()
            + usize::from(!group.is_empty()) * (group.len() + 1)
            + usize::from(!subject.is_empty()) * (subject.len() + 1),
    );

    if let Some(prefix) = prefix {
        full_subject.push_str(prefix);
        full_subject.push('.');
    }

    full_subject.push('v');
    full_subject.push_str(major);

    if !group.is_empty() {
        full_subject.push('.');
        full_subject.push_str(group);
    }

    if !subject.is_empty() {
        full_subject.push('.');
        full_subject.push_str(subject);
    }

    full_subject
}

#[cfg(test)]
mod tests {
    use super::build_subject;

    #[test]
    fn builds_subject_with_prefix() {
        assert_eq!(
            build_subject(Some("api"), "1.2.3", "math", "sum"),
            "api.v1.math.sum"
        );
        assert_eq!(
            build_subject(Some("api"), "1.2.3", "", "health"),
            "api.v1.health"
        );
    }

    #[test]
    fn builds_subject_without_prefix() {
        assert_eq!(build_subject(None, "0.9.1", "math", "sum"), "v0.math.sum");
        assert_eq!(build_subject(None, "0.9.1", "", "health"), "v0.health");
    }

    #[test]
    fn subject_version_uses_major_only() {
        assert_eq!(
            build_subject(Some("api"), "2.0.0", "live", "jobs"),
            "api.v2.live.jobs"
        );
        assert_eq!(
            build_subject(Some("api"), "2.9.99", "live", "jobs"),
            "api.v2.live.jobs"
        );
    }
}
