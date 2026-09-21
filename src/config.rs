use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub app: AppSection,
    pub database: DatabaseSection,
    pub redis: RedisSection,
    pub jwt: JwtSection,
    pub storage: StorageSection,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppSection {
    pub env: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseSection {
    pub url: String,
    pub max_connections: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RedisSection {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JwtSection {
    pub secret: String,
    pub access_expiry_seconds: i64,
    pub refresh_expiry_seconds: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageSection {
    pub backend: String,
    pub endpoint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub region: String,
    pub gif_cdn_url: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        dotenvy::dotenv().ok();

        let app = AppSection {
            env: std::env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            host: std::env::var("APP_HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port: std::env::var("APP_PORT")
                .unwrap_or_else(|_| "8080".into())
                .parse()?,
        };

        let database = DatabaseSection {
            url: std::env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL is required"))?,
            max_connections: std::env::var("DATABASE_MAX_CONNECTIONS")
                .unwrap_or_else(|_| "20".into())
                .parse()?,
        };

        let redis = RedisSection {
            url: std::env::var("REDIS_URL")
                .map_err(|_| anyhow::anyhow!("REDIS_URL is required"))?,
        };

        let jwt = JwtSection {
            secret: std::env::var("JWT_SECRET")
                .map_err(|_| anyhow::anyhow!("JWT_SECRET is required"))?,
            access_expiry_seconds: std::env::var("JWT_EXPIRY_SECONDS")
                .unwrap_or_else(|_| "3600".into())
                .parse()?,
            refresh_expiry_seconds: std::env::var("JWT_REFRESH_EXPIRY_SECONDS")
                .unwrap_or_else(|_| "2592000".into())
                .parse()?,
        };

        let storage = StorageSection {
            backend: std::env::var("STORAGE_BACKEND").unwrap_or_else(|_| "minio".into()),
            endpoint: std::env::var("S3_ENDPOINT")
                .map_err(|_| anyhow::anyhow!("S3_ENDPOINT is required"))?,
            access_key_id: std::env::var("S3_ACCESS_KEY_ID")
                .map_err(|_| anyhow::anyhow!("S3_ACCESS_KEY_ID is required"))?,
            secret_access_key: std::env::var("S3_SECRET_ACCESS_KEY")
                .map_err(|_| anyhow::anyhow!("S3_SECRET_ACCESS_KEY is required"))?,
            bucket: std::env::var("S3_BUCKET")
                .map_err(|_| anyhow::anyhow!("S3_BUCKET is required"))?,
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
            gif_cdn_url: std::env::var("GIF_CDN_URL").ok().filter(|s| !s.is_empty()),
        };

        Ok(Self {
            app,
            database,
            redis,
            jwt,
            storage,
        })
    }

    pub fn is_production(&self) -> bool {
        self.app.env == "production"
    }
}
