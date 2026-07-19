use crate::error::AppError;
use std::future::Future;
use std::time::Duration;

pub async fn initialize_with_timeout<T, F>(
    timeout_ms: u64,
    initialization: F,
) -> Result<T, AppError>
where
    F: Future<Output = Result<T, AppError>>,
{
    tokio::time::timeout(Duration::from_millis(timeout_ms), initialization)
        .await
        .map_err(|_| AppError::ComponentInitializationTimeout)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_initialization_returns_value() {
        let result = tauri::async_runtime::block_on(initialize_with_timeout(100, async {
            Ok::<_, AppError>(42)
        }));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn initialization_error_is_preserved() {
        let result = tauri::async_runtime::block_on(initialize_with_timeout(100, async {
            Err::<(), _>(AppError::ComponentInitializationFailure(
                "failed safely".into(),
            ))
        }));
        assert!(matches!(
            result,
            Err(AppError::ComponentInitializationFailure(_))
        ));
    }

    #[test]
    fn slow_initialization_times_out() {
        let result = tauri::async_runtime::block_on(initialize_with_timeout(1, async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            Ok::<_, AppError>(())
        }));
        assert!(matches!(
            result,
            Err(AppError::ComponentInitializationTimeout)
        ));
    }
}
