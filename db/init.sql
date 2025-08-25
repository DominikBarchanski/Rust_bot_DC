-- Base tables (no unique on (raid_id, user_id) — we allow mains + multiple alts)
CREATE TABLE IF NOT EXISTS raids (
    id UUID PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    message_id BIGINT NOT NULL,
    scheduled_for TIMESTAMPTZ NOT NULL,
    created_by BIGINT NOT NULL,
    owner_id BIGINT NOT NULL,
    description TEXT NOT NULL,
    is_priority BOOLEAN NOT NULL DEFAULT FALSE,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    priority_list JSONB NOT NULL DEFAULT '[]'
);

CREATE TABLE IF NOT EXISTS raid_participants (
    id UUID PRIMARY KEY,
    raid_id UUID NOT NULL,
    user_id BIGINT NOT NULL,
    is_main BOOLEAN NOT NULL DEFAULT FALSE,
    joined_as TEXT NOT NULL,
    is_reserve BOOLEAN NOT NULL DEFAULT FALSE,
    joined_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    is_alt BOOLEAN NOT NULL DEFAULT FALSE,
    FOREIGN KEY (raid_id) REFERENCES raids(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_raids_guild_time ON raids (guild_id, scheduled_for);
CREATE INDEX IF NOT EXISTS idx_participants_raid ON raid_participants (raid_id);
CREATE INDEX IF NOT EXISTS idx_participants_main_order ON raid_participants (raid_id, is_main DESC, is_alt ASC, joined_at ASC);

ALTER TABLE raids
  ADD COLUMN IF NOT EXISTS raid_name TEXT NOT NULL DEFAULT 'arma_v2',
  ADD COLUMN IF NOT EXISTS max_players INT NOT NULL DEFAULT 12,
  ADD COLUMN IF NOT EXISTS allow_alts BOOLEAN NOT NULL DEFAULT TRUE,
  ADD COLUMN IF NOT EXISTS max_alts INT NOT NULL DEFAULT 1,
  ADD COLUMN IF NOT EXISTS priority_role_id BIGINT,
  ADD COLUMN IF NOT EXISTS priority_role_ids BIGINT[] NULL,
  ADD COLUMN IF NOT EXISTS priority_until TIMESTAMPTZ;

ALTER TABLE raid_participants
  ADD COLUMN IF NOT EXISTS tag_suffix TEXT NOT NULL DEFAULT '';
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'raids_raid_name_check'
  ) THEN
    ALTER TABLE raids
      ADD CONSTRAINT raids_raid_name_check CHECK (raid_name IN ('ArmaV2','Pollutus','Arma','Azgobas','Valehir','Alzanor','Hc_Azgobas','Hc_Valehir','Hc_Alzanor','Hc_A8-A6','Hc_A1-A5'));
  END IF;
END $$;

CREATE INDEX IF NOT EXISTS idx_raids_priority_until ON raids (priority_until);

-- If you previously had UNIQUE(raid_id,user_id), drop it
ALTER TABLE raid_participants DROP CONSTRAINT IF EXISTS raid_participants_raid_id_user_id_key;

