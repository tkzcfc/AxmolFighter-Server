ALTER TABLE characters
    ADD COLUMN IF NOT EXISTS hair_fashion_id INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS clothes_fashion_id INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS skin_fashion_id INT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS character_fashions (
    id           BIGSERIAL PRIMARY KEY,
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    config_id    INT NOT NULL,
    position     INT NOT NULL DEFAULT 0,
    worn         BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_character_fashions_character ON character_fashions (character_id);
