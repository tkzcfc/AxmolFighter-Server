-- 账号表
CREATE TABLE IF NOT EXISTS accounts (
    id         BIGSERIAL PRIMARY KEY,
    account    VARCHAR(64) NOT NULL UNIQUE,
    password   VARCHAR(128) NOT NULL,
    nickname   VARCHAR(64) NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_accounts_account ON accounts (account);
