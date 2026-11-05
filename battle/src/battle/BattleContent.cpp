#include "battle/BattleContent.h"

#include "mugen/avatar/data/AvatarAssetCache.h"
#include "mugen/conf/ReflectedConfig.h"
#include "mugen/core/io/ContentMount.h"
#include "mugen/core/io/FileUtils.h"

#include <spdlog/spdlog.h>

namespace battle
{

bool mountBattleContent(const std::string& contentRoot)
{
    if (contentRoot.empty())
        return false;

    static vfs::VirtualFileSystem contentFs;
    if (mugen::io::mountContentRoot(contentFs, contentRoot) != vfs::Error::OK)
    {
        spdlog::error("Failed to mount / from content_root={}", contentRoot);
        return false;
    }
    mugen::io::setVfs(&contentFs);

    if (!mugen::conf::Config::instance().load("/mugen/config/config.bin"))
    {
        spdlog::error("Failed to load config.bin from content_root={}", contentRoot);
        return false;
    }
    auto* avatarCache = mugen::AvatarAssetCache::getInstance();
    if (!avatarCache->load("/mugen/config/avatar.bin"))
    {
        spdlog::error("Failed to load avatar.bin from content_root={}", contentRoot);
        return false;
    }
    spdlog::info("Battle content mounted from {}", contentRoot);
    return true;
}

}  // namespace battle
