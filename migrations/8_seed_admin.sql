-- Password: "changeme" (Argon2id). Replace hash before non-local use.
INSERT INTO users (email, password_hash, display_name, role, email_verified)
VALUES (
    'admin@example.com',
    '$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzYWx0$REPLACE_WITH_REAL_HASH',
    'Admin',
    'admin',
    true
)
ON CONFLICT (lower(email)) DO NOTHING;