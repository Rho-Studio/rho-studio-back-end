use super::{ObjectStorage, StorageError};
use async_trait::async_trait;
use bytes::Bytes;
use s3::bucket::Bucket;
use s3::bucket_ops::BucketConfiguration;
use s3::creds::Credentials;
use s3::region::Region;
use std::sync::Arc;

pub struct S3Storage {
    bucket: Arc<Bucket>,
    cdn_base: Option<String>,
    endpoint: String,
    access_key_id: String,
    secret_access_key: String,
    bucket_name: String,
    region: String,
}

impl S3Storage {
    pub fn new(
        endpoint: &str,
        access_key_id: &str,
        secret_access_key: &str,
        bucket_name: &str,
        region: &str,
        cdn_base: Option<String>,
    ) -> Result<Self, StorageError> {
        let region = Region::Custom {
            region: region.to_string(),
            endpoint: endpoint.to_string(),
        };

        let creds = Credentials::new(
            Some(access_key_id),
            Some(secret_access_key),
            None,
            None,
            None,
        )
        .map_err(|e| StorageError::Backend(e.to_string()))?;

        let bucket = Bucket::new(bucket_name, region.clone(), creds)
            .map_err(|e| StorageError::Backend(e.to_string()))?
            .with_path_style();

        Ok(Self {
            bucket: Arc::new(bucket),
            cdn_base: cdn_base.map(|s| s.trim_end_matches('/').to_string()),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            access_key_id: access_key_id.to_string(),
            secret_access_key: secret_access_key.to_string(),
            bucket_name: bucket_name.to_string(),
            region: region.to_string(),
        })
    }

    pub async fn ensure_bucket(&self) -> Result<(), StorageError> {
        let region = Region::Custom {
            region: self.region.clone(),
            endpoint: self.endpoint.clone(),
        };
        let credentials = Credentials::new(
            Some(&self.access_key_id),
            Some(&self.secret_access_key),
            None,
            None,
            None,
        )
        .map_err(|e| StorageError::Backend(e.to_string()))?;

        let response = Bucket::create_with_path_style(
            &self.bucket_name,
            region,
            credentials,
            BucketConfiguration::default(),
        )
        .await
        .map_err(|e| StorageError::Backend(e.to_string()))?;

        if (200..300).contains(&response.response_code) || response.response_code == 409 {
            Ok(())
        } else {
            Err(StorageError::Backend(format!(
                "bucket initialization returned status {}",
                response.response_code
            )))
        }
    }
}

#[async_trait]
impl ObjectStorage for S3Storage {
    async fn put_object(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> Result<(), StorageError> {
        let data = Bytes::from(bytes);
        let response = self
            .bucket
            .put_object_with_content_type(key, &data, content_type)
            .await
            .map_err(|e| StorageError::PutFailed(e.to_string()))?;

        if (200..300).contains(&response.status_code()) {
            Ok(())
        } else {
            Err(StorageError::PutFailed(format!(
                "unexpected status {}",
                response.status_code()
            )))
        }
    }

    async fn get_object(&self, key: &str) -> Result<Vec<u8>, StorageError> {
        let response = self
            .bucket
            .get_object(key)
            .await
            .map_err(|e| StorageError::GetFailed(e.to_string()))?;

        if response.status_code() == 404 {
            return Err(StorageError::NotFound(key.to_string()));
        }
        if (200..300).contains(&response.status_code()) {
            Ok(response.bytes().to_vec())
        } else {
            Err(StorageError::GetFailed(format!(
                "unexpected status {}",
                response.status_code()
            )))
        }
    }

    async fn delete_object(&self, key: &str) -> Result<(), StorageError> {
        let response = self
            .bucket
            .delete_object(key)
            .await
            .map_err(|e| StorageError::DeleteFailed(e.to_string()))?;

        if (200..300).contains(&response.status_code()) {
            Ok(())
        } else {
            Err(StorageError::DeleteFailed(format!(
                "unexpected status {}",
                response.status_code()
            )))
        }
    }

    fn public_url(&self, key: &str) -> String {
        let key = key.trim_start_matches('/');
        match &self.cdn_base {
            Some(base) => format!("{}/{}", base, key),
            None => {
                let bucket = self.bucket.name();
                format!("{}/{}/{}", self.endpoint, bucket, key)
            }
        }
    }

    async fn presigned_put_url(
        &self,
        key: &str,
        expires_secs: u32,
    ) -> Result<String, StorageError> {
        let url = self
            .bucket
            .presign_put(key, expires_secs, None)
            .await
            .map_err(|e| StorageError::PresignFailed(e.to_string()))?;
        Ok(url)
    }
}
