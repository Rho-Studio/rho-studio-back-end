mod s3_storage;
pub use s3_storage::S3Storage;

use async_trait::async_trait;
use thiserror::Error;

#[expect(
    dead_code,
    reason = "storage errors are used as storage operations are added"
)]
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("object not found: {0}")]
    NotFound(String),

    #[error("put failed: {0}")]
    PutFailed(String),

    #[error("get failed: {0}")]
    GetFailed(String),

    #[error("delete failed: {0}")]
    DeleteFailed(String),

    #[error("presign failed: {0}")]
    PresignFailed(String),

    #[error("storage backend error: {0}")]
    Backend(String),
}

#[expect(
    dead_code,
    reason = "storage operations are exposed for upcoming routes"
)]
#[async_trait]
pub trait ObjectStorage: Send + Sync {
    async fn put_object(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError>;

    async fn get_object(&self, key: &str) -> Result<Vec<u8>, StorageError>;

    async fn delete_object(&self, key: &str) -> Result<(), StorageError>;

    fn public_url(&self, key: &str) -> String;

    async fn presigned_put_url(&self, key: &str, expires_secs: u32)
    -> Result<String, StorageError>;
}
