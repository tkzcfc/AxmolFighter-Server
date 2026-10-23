-- Persist fashion wear (table already exists), armament, skill slots, pet, partner.

ALTER TABLE characters
    ADD COLUMN IF NOT EXISTS armament_appearance_id INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS active_pet_id INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS active_partner_id INT NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS character_armaments (
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    config_id    INT NOT NULL,
    level        INT NOT NULL DEFAULT 1,
    star         INT NOT NULL DEFAULT 0,
    grade        INT NOT NULL DEFAULT 0,
    skin_id      INT NOT NULL DEFAULT 0,
    skill_id     INT NOT NULL DEFAULT 0,
    PRIMARY KEY (character_id, config_id)
);

CREATE TABLE IF NOT EXISTS character_skill_slots (
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    slot         INT NOT NULL,
    leaf_id      INT NOT NULL DEFAULT 0,
    PRIMARY KEY (character_id, slot)
);

CREATE TABLE IF NOT EXISTS character_pets (
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    config_id    INT NOT NULL,
    level        INT NOT NULL DEFAULT 1,
    star         INT NOT NULL DEFAULT 0,
    PRIMARY KEY (character_id, config_id)
);

CREATE TABLE IF NOT EXISTS character_partners (
    character_id BIGINT NOT NULL REFERENCES characters(id) ON DELETE CASCADE,
    config_id    INT NOT NULL,
    level        INT NOT NULL DEFAULT 1,
    star         INT NOT NULL DEFAULT 0,
    PRIMARY KEY (character_id, config_id)
);
