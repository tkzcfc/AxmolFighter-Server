-- 服务器配置
CREATE TABLE IF NOT EXISTS server_settings (
    key       VARCHAR(64) PRIMARY KEY,
    value_int BIGINT NOT NULL
);

-- 角色表
CREATE TABLE IF NOT EXISTS characters (
    id         BIGSERIAL PRIMARY KEY,
    player_id  BIGINT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    name       VARCHAR(64) NOT NULL UNIQUE,
    class_id   INT NOT NULL DEFAULT 0,
    gender     INT NOT NULL DEFAULT 0,
    level      INT NOT NULL DEFAULT 1,
    exp        BIGINT NOT NULL DEFAULT 0,
    gold       BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_characters_player_id ON characters (player_id);

-- 装备表
CREATE TABLE IF NOT EXISTS equipments (
    id                 BIGSERIAL PRIMARY KEY,
    owner_character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    config_id          BIGINT NOT NULL DEFAULT 0,
    enhance_level      INT NOT NULL DEFAULT 0,
    refine_level       INT NOT NULL DEFAULT 0,
    enchant_props_json TEXT NOT NULL DEFAULT '[]',
    slot               INT NOT NULL DEFAULT -1,
    in_bag             BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_equipments_owner ON equipments (owner_character_id);

-- 背包道具表
CREATE TABLE IF NOT EXISTS inventory_items (
    id           BIGSERIAL PRIMARY KEY,
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    config_id    BIGINT NOT NULL,
    count        INT NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_inventory_items_character ON inventory_items (character_id);

-- 默认服务器参数
INSERT INTO server_settings (key, value_int)
VALUES ('max_character_count', 10)
ON CONFLICT (key) DO NOTHING;
