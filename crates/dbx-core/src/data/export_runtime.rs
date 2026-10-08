//! Execution context for long-running export tasks.

use std::future::Future;

/// Spawns a long-running export task that interleaves async database fetches
/// with synchronous row formatting and buffered disk writes.
///
/// Running such a task directly on a tokio worker (`tokio::spawn`) blocks that
/// worker for the whole synchronous write slice of every page, so a multi-GB
/// export stalls every concurrent command sharing the runtime. This helper
/// instead runs the task on the blocking pool and drives it with
/// [`tokio::runtime::Handle::block_on`]: the blocking thread owns the
/// synchronous formatting and writes while async fetches still execute on the
/// regular workers. The blocking pool grows on demand, so concurrent exports
/// and other `spawn_blocking` users do not starve each other.
pub fn spawn_export_task(task: impl Future<Output = ()> + Send + 'static) {
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(task));
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// The task must run to completion from a blocking-pool thread: async
    /// timers, `tokio::fs`, and nested `spawn_blocking` all need the runtime
    /// context that `Handle::block_on` provides there.
    #[tokio::test]
    async fn export_task_runs_async_and_blocking_work_to_completion() {
        let (tx, rx) = tokio::sync::oneshot::channel::<u64>();
        super::spawn_export_task(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let nested = tokio::task::spawn_blocking(|| 40u64 + 2).await.unwrap_or(0);
            tx.send(nested).expect("receiver dropped");
        });
        let value = tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .expect("export task did not finish")
            .expect("export task dropped its sender");
        assert_eq!(value, 42);
    }
}
