CREATE TABLE user_devices (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id       UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_token  TEXT NOT NULL,
    platform      TEXT NOT NULL CHECK (platform IN ('ios', 'android', 'web')),
    locale        TEXT,
    app_version   TEXT,
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (user_id, device_token)
);

CREATE INDEX idx_user_devices_user_id      ON user_devices(user_id);
CREATE INDEX idx_user_devices_platform     ON user_devices(platform);
CREATE INDEX idx_user_devices_last_seen_at ON user_devices(last_seen_at DESC);