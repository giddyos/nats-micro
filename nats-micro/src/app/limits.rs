use anyhow::Result;

pub(super) const DEFAULT_CONCURRENCY_LIMIT: u64 = 512;
pub(super) const DEFAULT_MAX_CONCURRENCY_LIMIT: u64 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedConsumerConcurrencyLimit {
    pub(super) value: u64,
    pub(super) promoted_from_default: bool,
}

pub(super) fn resolve_endpoint_concurrency_limit(
    requested_limit: Option<u64>,
    default_limit: u64,
    max_limit: u64,
) -> Result<u64> {
    if let Some(limit) = requested_limit {
        anyhow::ensure!(
            limit <= max_limit,
            "requested concurrency_limit ({limit}) cannot exceed max_concurrency_limit ({max_limit})"
        );
        Ok(limit)
    } else {
        Ok(default_limit)
    }
}

pub(super) fn validate_consumer_concurrency_limit(limit: u64, max_ack_pending: i64) -> Result<()> {
    if max_ack_pending > 0 {
        let max_ack_pending = u64::try_from(max_ack_pending).unwrap_or(u64::MAX);
        anyhow::ensure!(
            limit <= max_ack_pending,
            "consumer concurrency_limit ({limit}) cannot exceed max_ack_pending ({max_ack_pending})"
        );
    }

    Ok(())
}

pub(super) fn resolve_consumer_concurrency_limit(
    requested_limit: Option<u64>,
    server_max_ack_pending: i64,
    default_limit: u64,
    max_limit: u64,
    promote_server_max_ack_pending: bool,
) -> ResolvedConsumerConcurrencyLimit {
    if let Some(limit) = requested_limit {
        debug_assert!(
            limit <= max_limit,
            "requested consumer concurrency_limit must be validated before resolving"
        );
        ResolvedConsumerConcurrencyLimit {
            value: limit,
            promoted_from_default: false,
        }
    } else {
        let promoted_limit = promote_server_max_ack_pending
            .then(|| u64::try_from(server_max_ack_pending).ok())
            .flatten()
            .filter(|limit| *limit > default_limit);

        match promoted_limit {
            Some(limit) => ResolvedConsumerConcurrencyLimit {
                value: limit.min(max_limit),
                promoted_from_default: true,
            },
            None => ResolvedConsumerConcurrencyLimit {
                value: default_limit,
                promoted_from_default: false,
            },
        }
    }
}

pub(super) fn validate_requested_concurrency_limit(limit: u64, max_limit: u64) -> Result<()> {
    anyhow::ensure!(
        limit <= max_limit,
        "requested concurrency_limit ({limit}) cannot exceed max_concurrency_limit ({max_limit})"
    );
    Ok(())
}

pub(super) fn semaphore_permits(limit: u64) -> usize {
    usize::try_from(limit).unwrap_or(usize::MAX)
}
