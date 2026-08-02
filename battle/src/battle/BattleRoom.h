#pragma once

#include "battle/BattleTypes.h"

#include <cstdint>
#include <memory>
#include <string>
#include <unordered_map>
#include <utility>

namespace mugen
{
class GameWord;
}

namespace PB::Types
{
class BattlePlayerSpec;
}

namespace battle
{

// 单场战斗实例：拥有 GameWord，负责玩家入座、输入写入、固定帧推进与快照序列化。
class BattleRoom
{
public:
    BattleRoom();
    ~BattleRoom();

    bool init(const BattleRoomConfig& config);
    void shutdown();

    bool addPlayer(const PB::Types::BattlePlayerSpec& spec);
    void removePlayer(std::uint32_t sessionId);
    bool hasPlayer(std::uint32_t sessionId) const;
    bool isFull() const;
    bool isEmpty() const;

    void applyInput(std::uint32_t sessionId, std::uint32_t clientFrame, std::uint32_t inputMask);
    void tick(float fixedDt);

    std::string serializeWorld() const;

    std::uint32_t battleId() const { return m_config.battleId; }
    std::uint32_t serverFrame() const { return m_serverFrame; }
    std::int32_t mapId() const { return m_config.mapId; }
    float elapsed() const { return m_elapsed; }

    const std::unordered_map<std::uint32_t, PlayerSlot>& players() const { return m_players; }
    std::uint32_t lastClientFrame(std::uint32_t sessionId) const;
    mugen::EntityId actorIdForSession(std::uint32_t sessionId) const;

private:
    std::pair<std::int32_t, std::int32_t> resolveSpawnPoint(std::size_t slotIndex) const;

private:
    BattleRoomConfig m_config;
    std::unique_ptr<mugen::GameWord> m_world;
    std::unordered_map<std::uint32_t, PlayerSlot> m_players;
    std::uint32_t m_serverFrame = 0;
    float m_elapsed = 0.0f;
};

}  // namespace battle
