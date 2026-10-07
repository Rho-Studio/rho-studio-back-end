/**
 * ============================================================================
 * ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
 * ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
 * ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
 * ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
 * https://rho.studio/
 * ============================================================================
 * File:        src/config.rs
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-07
 * ============================================================================
 * Description:
 *      Typed application configuration loaded from environment variables.
 *      Every section is validated at load time; the process refuses to start
 *      with an incomplete or unsafe configuration. There are no silent
 *      defaults for production-critical values.
 *
 *      Sections:
 *          - AppSection            Environment name, bind host, bind port.
 *          - AuthRateLimitSection  Window, per-action limits, trusted proxies.
 *          - DatabaseSection       Postgres URL and pool size.
 *          - RedisSection          Redis URL (disposable cache / rate limit).
 *          - JwtSection            HS256 secret and access / refresh lifetimes.
 *          - StorageSection        S3-compatible endpoint and credentials.
 *
 *      Top-level fields:
 *          - app                   APP_ENV / APP_HOST / APP_PORT.
 *          - cors_allowed_origins  Comma-separated origins; validated for
 *                                  scheme and format before the process
 *                                  binds to a socket.
 *          - auth_rate_limits      Redis-backed auth limiter configuration.
 *          - database              Postgres connection settings.
 *          - redis                 Redis connection URL.
 *          - jwt                   Access and refresh token policy.
 *          - storage               Object storage backend configuration.
 *
 *      Depends on:
 *          - dotenvy               Loads .env in development.
 *          - ipnet                 Parses trusted_proxy_cidrs as IP networks.
 *          - anyhow                Aggregates validation errors at load time.
 *
 *      Design notes:
 *          - Fail fast. Every required variable is checked at startup.
 *            Missing or malformed values abort the process with a clear
 *            message rather than surfacing as a runtime 500 later.
 *          - Environment-aware validation. Production enforces stricter
 *            rules than development: non-empty CORS allowlist, HTTPS-only
 *            origins, non-empty trusted proxy CIDRs, and a JWT secret of at
 *            least 32 bytes that is not the placeholder value.
 *          - CORS allowlist is never empty in production. An empty list
 *            would silently allow no cross-origin traffic; the validator
 *            rejects it explicitly so the operator notices at boot.
 *          - Wildcard CORS origins are rejected. The is_valid_cors_origin
 *            helper refuses "*", paths, queries, fragments, and userinfo,
 *            and requires HTTPS in production.
 *          - JWT expiry bounds are enforced. Access ≤ 3600 s; refresh
 *            ≤ 7,776,000 s (90 days). Prevents a misconfiguration that
 *            would issue long-lived access tokens.
 *          - Rate-limit values are bounded. window_seconds ∈ [1, 86400];
 *            attempt counts must be positive.
 *          - No secrets are logged. Errors reference variable names, not
 *            values.
 *
 *      Known gaps (tracked in DataModel.md and DevPlan.md):
 *          - JWT_SECRET is a single static secret. There is no kid-based
 *            rotation window; rotating it invalidates every outstanding
 *            token. Tracked as HU-007.
 *          - Trusted proxy CIDRs are parsed but not range-validated beyond
 *            syntactic correctness; operators must supply sensible ranges.
 *          - No config hot-reload. Changes require a process restart.
 *          - No KMS / secret-manager integration. Production secrets are
 *            expected to be injected via the environment by the deployment
 *            platform.
 *
 * ============================================================================
 */
use ipnet::IpNet;
use serde::Deserialize;

fn is_valid_cors_origin(origin: &str, production: bool) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    if authority.is_empty()
        || authority == "*"
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains('@')
    {
        return false;
    }
    if scheme != "https" && (production || scheme != "http") {
        return false;
    }
    authority.parse::<axum::http::uri::Authority>().is_ok()
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    pub app: AppSection,
    pub cors_allowed_origins: Vec<String>,
    pub auth_rate_limits: AuthRateLimitSection,
    pub database: DatabaseSection,
    pub redis: RedisSection,
    pub jwt: JwtSection,
    pub storage: StorageSection,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthRateLimitSection {
    pub window_seconds: u64,
    pub login_attempts: u32,
    pub refresh_attempts: u32,
    pub trusted_proxy_cidrs: Vec<IpNet>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppSection {
    pub env: String,
    pub host: String,
    pub port: u16,
}

impl AppSection {
    pub fn is_production(&self) -> bool {
        self.env == "production"
    }
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
    pub auto_create_bucket: bool,
    pub endpoint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub bucket: String,
    pub region: String,
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

        let default_cors_origins = if app.env == "development" {
            "http://localhost:3000,http://127.0.0.1:3000"
        } else {
            ""
        };
        let cors_allowed_origins = std::env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_else(|_| default_cors_origins.to_string())
            .split(',')
            .map(str::trim)
            .filter(|origin| !origin.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if app.is_production() && cors_allowed_origins.is_empty() {
            anyhow::bail!("CORS_ALLOWED_ORIGINS must be configured in production");
        }
        if cors_allowed_origins
            .iter()
            .any(|origin| !is_valid_cors_origin(origin, app.is_production()))
        {
            anyhow::bail!(
                "CORS_ALLOWED_ORIGINS must contain explicit origins without wildcards or paths; production origins must use HTTPS"
            );
        }

        let auth_rate_limits = AuthRateLimitSection {
            window_seconds: std::env::var("AUTH_RATE_LIMIT_WINDOW_SECONDS")
                .unwrap_or_else(|_| "900".into())
                .parse()?,
            login_attempts: std::env::var("AUTH_LOGIN_RATE_LIMIT_ATTEMPTS")
                .unwrap_or_else(|_| "10".into())
                .parse()?,
            refresh_attempts: std::env::var("AUTH_REFRESH_RATE_LIMIT_ATTEMPTS")
                .unwrap_or_else(|_| "30".into())
                .parse()?,
            trusted_proxy_cidrs: std::env::var("TRUSTED_PROXY_CIDRS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|cidr| !cidr.is_empty())
                .map(str::parse)
                .collect::<Result<Vec<_>, _>>()?,
        };
        if auth_rate_limits.window_seconds == 0
            || auth_rate_limits.window_seconds > 86_400
            || auth_rate_limits.login_attempts == 0
            || auth_rate_limits.refresh_attempts == 0
        {
            anyhow::bail!("Authentication rate-limit settings must be positive and bounded");
        }
        if app.is_production() && auth_rate_limits.trusted_proxy_cidrs.is_empty() {
            anyhow::bail!("TRUSTED_PROXY_CIDRS must be configured in production");
        }

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
        if jwt.access_expiry_seconds <= 0 || jwt.access_expiry_seconds > 3600 {
            anyhow::bail!("JWT_EXPIRY_SECONDS must be between 1 and 3600");
        }
        if jwt.refresh_expiry_seconds <= 0 || jwt.refresh_expiry_seconds > 7_776_000 {
            anyhow::bail!("JWT_REFRESH_EXPIRY_SECONDS must be between 1 and 7776000");
        }
        if app.is_production() && (jwt.secret.len() < 32 || jwt.secret == "change-me-in-production")
        {
            anyhow::bail!("JWT_SECRET must be a production-grade secret of at least 32 bytes");
        }

        let storage = StorageSection {
            backend: std::env::var("STORAGE_BACKEND").unwrap_or_else(|_| "s3".into()),
            auto_create_bucket: std::env::var("STORAGE_AUTO_CREATE_BUCKET")
                .unwrap_or_else(|_| "false".into())
                .parse()?,
            endpoint: std::env::var("S3_ENDPOINT")
                .map_err(|_| anyhow::anyhow!("S3_ENDPOINT is required"))?,
            access_key_id: std::env::var("S3_ACCESS_KEY_ID")
                .map_err(|_| anyhow::anyhow!("S3_ACCESS_KEY_ID is required"))?,
            secret_access_key: std::env::var("S3_SECRET_ACCESS_KEY")
                .map_err(|_| anyhow::anyhow!("S3_SECRET_ACCESS_KEY is required"))?,
            bucket: std::env::var("S3_BUCKET")
                .map_err(|_| anyhow::anyhow!("S3_BUCKET is required"))?,
            region: std::env::var("S3_REGION").unwrap_or_else(|_| "us-east-1".into()),
        };

        Ok(Self {
            app,
            cors_allowed_origins,
            auth_rate_limits,
            database,
            redis,
            jwt,
            storage,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::is_valid_cors_origin;

    #[test]
    fn cors_origins_must_be_explicit_and_production_origins_must_use_https() {
        assert!(is_valid_cors_origin("https://app.example.mx", true));
        assert!(is_valid_cors_origin("http://localhost:3000", false));
        assert!(!is_valid_cors_origin("*", false));
        assert!(!is_valid_cors_origin("https://app.example.mx/path", true));
        assert!(!is_valid_cors_origin("http://app.example.mx", true));
    }
}
