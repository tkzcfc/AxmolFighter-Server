#include "battle/BattleSync.h"

#include "framework/BackendClient.h"

namespace battle
{

BattleSync::BattleSync(BackendClient& backend)
    : m_backend(backend)
{
}

void BattleSync::sendSnapshot(const BattleRoom& room)
{
    if (room.isEmpty())
        return;

    const std::string worldDump = room.serializeWorld();

    for (const auto& [sessionId, slot] : room.players())
    {
        PB::Battle::BattleSnapshotPush push;
        push.set_battle_id(room.battleId());
        push.set_server_frame(room.serverFrame());
        push.set_server_time_ms(static_cast<std::uint64_t>(room.elapsed() * 1000.0f));
        push.set_world_dump(worldDump);
        push.set_last_processed_client_frame(slot.lastClientFrame);

        m_backend.sendPush(sessionId, push);
    }
}

}  // namespace battle
