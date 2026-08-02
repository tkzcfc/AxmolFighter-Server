#pragma once

#include "battle/BattleRoom.h"
#include "client_battle.pb.h"

#include <cstdint>

namespace battle
{

class BackendClient;

// 负责战斗快照的封装与下发。
class BattleSync
{
public:
    explicit BattleSync(BackendClient& backend);

    void sendSnapshot(const BattleRoom& room);

private:
    BackendClient& m_backend;
};

}  // namespace battle
