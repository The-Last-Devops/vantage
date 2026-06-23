-- Singleton row holding hub-wide settings (currently the S3 backup target).
CREATE TABLE app_settings (
    id smallint PRIMARY KEY DEFAULT 1,
    s3 jsonb,
    CONSTRAINT app_settings_singleton CHECK (id = 1)
);
INSERT INTO app_settings (id, s3) VALUES (1, NULL) ON CONFLICT (id) DO NOTHING;
