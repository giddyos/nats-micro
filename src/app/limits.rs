use anyhow::Result;

pub(super) const DEFAULT_CONCURRENCY_LIMIT: u64 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ResolvedConsumerConcurrencyLimit {
    pub(super) value: u64,
    pub(super) promoted_from_default: bool,
}

pub(super) fn resolve_endpoint_concurrency_limit(
    requested_limit: Option<u64>,
    default_limit: u64,
) -> u64 {
    requested_limit.unwrap_or(default_limit)
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
) -> ResolvedConsumerConcurrencyLimit {
    match requested_limit {
        Some(limit) => ResolvedConsumerConcurrencyLimit {
            value: limit,
            promoted_from_default: false,
        },
        None => {
            let promoted_limit = u64::try_from(server_max_ack_pending)
                .ok()
                .filter(|limit| *limit > default_limit);

            match promoted_limit {
                Some(limit) => ResolvedConsumerConcurrencyLimit {
                    value: limit,
                    promoted_from_default: true,
                },
                None => ResolvedConsumerConcurrencyLimit {
                    value: default_limit,
                    promoted_from_default: false,
                },
            }
        }
    }
}

pub(super) fn semaphore_permits(limit: u64) -> usize {
    usize::try_from(limit).unwrap_or(usize::MAX)
}
