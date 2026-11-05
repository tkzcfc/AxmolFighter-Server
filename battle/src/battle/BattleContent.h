#pragma once

#include <string>

namespace battle
{

// 挂载 Content 根目录到 VFS 虚根并加载 config.bin / avatar.bin。
// 供 BattleServer 与无网络单测复用。失败返回 false。
bool mountBattleContent(const std::string& contentRoot);

}  // namespace battle
