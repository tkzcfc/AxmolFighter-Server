#include "mugen/Components.h"
#include "mugen/GameWord.h"
#include "mugen/avatar/MotionPlayback.h"
#include "mugen/avatar/data/AvatarAssetCache.h"
#include "mugen/conf/Config.h"
#include "mugen/conf/GameDef.h"
#include "mugen/core/io/FileUtils.h"
#include "mugen/resource/ResourceUrl.h"

#include <cstdint>
#include <filesystem>
#include <iostream>
#include <string>

namespace fs = std::filesystem;
using namespace mugen;

namespace
{

struct FightResult
{
    bool hit          = false;
    uint32_t hitFrame = 0;
    std::string defenderStateAfterHit;
    std::string fingerprint;
};

Entity* spawnSwordman(GameWord& world, float x, float y, FacingDirection facing)
{
    const char* actorPaths[] = {
        "mugen/config/character/swordman/swordman.act",
    };
    const ActorConfig* actorCfg = nullptr;
    for (const char* path : actorPaths)
    {
        actorCfg = Config::getInstance()->getActorConfig(path);
        if (actorCfg)
            break;
    }
    if (!actorCfg)
    {
        std::cerr << "spawnSwordman: ActorConfig not found\n";
        return nullptr;
    }

    const ChrConfig* chrConfig = Config::getInstance()->getChrConfig(actorCfg->character);
    if (!chrConfig)
    {
        std::cerr << "spawnSwordman: ChrConfig not found: " << actorCfg->character << "\n";
        return nullptr;
    }

    Entity* player          = world.ecsManager.newEntity();
    auto* avatarComp        = MG_ADD_COMPONENT(player, AvatarComponent);
    MG_ADD_COMPONENT(player, AvatarRenderComponent);
    MG_ADD_COMPONENT(player, InputComponent);
    auto* physicsComp       = MG_ADD_COMPONENT(player, PhysicsComponent);
    auto* transformComp     = MG_ADD_COMPONENT(player, TransformComponent);
    auto* identityComp      = MG_ADD_COMPONENT(player, IdentityComponent);
    auto* equipmentComp     = MG_ADD_COMPONENT(player, EquipmentComponent);
    auto* skillBarComp      = MG_ADD_COMPONENT(player, SkillBarComponent);
    MG_ADD_COMPONENT(player, SkillStateComponent);
    auto* attributeComp     = MG_ADD_COMPONENT(player, AttributeComponent);
    auto* actorDataComp     = MG_ADD_COMPONENT(player, ActorDataComponent);
    MG_ADD_COMPONENT(player, SoundComponent);
    MG_ADD_COMPONENT(player, StatesMachineComponent);

    avatarComp->actorConfig = actorCfg;
    avatarComp->chrConfig   = chrConfig;

    for (auto& equipment : actorCfg->equipments)
        equipmentComp->equipments[equipment.type].configId = equipment.id;

    for (auto& skill : chrConfig->skillPool)
    {
        auto* skillConfig = Config::getInstance()->getSkillConfig(skill.path);
        if (!skillConfig)
            continue;
        if (!actorDataComp->hasSkill(skill.path))
        {
            SkillInstanceData data;
            data.level = 1;
            for (auto& actorSkill : actorCfg->skills)
            {
                if (actorSkill.name == skill.name)
                {
                    data.level = std::max(1, actorSkill.level);
                    break;
                }
            }
            data.buildFromConfig(skillConfig);
            actorDataComp->skills.push_back(data);
        }
    }

    for (auto& slotSkill : actorCfg->skillSlots)
    {
        SkillSlotItem slotItem;
        slotItem.slotIndex = slotSkill.slotIndex;
        for (auto& skillName : slotSkill.skillNames)
        {
            for (auto& skillItem : chrConfig->skillPool)
            {
                if (skillItem.name != skillName)
                    continue;
                auto skillIndex = actorDataComp->getSkillIndex(skillItem.path);
                if (skillIndex >= 0)
                    slotItem.skillIndexs.push_back(skillIndex);
                break;
            }
        }
        skillBarComp->skillSlots.push_back(slotItem);
    }

    attributeComp->baseAttribute    = chrConfig->attribute;
    attributeComp->currentAttribute = attributeComp->baseAttribute;

    transformComp->position.x      = static_cast<int32_t>(x);
    transformComp->position.y      = static_cast<int32_t>(y);
    transformComp->scale.x         = 1.0f;
    transformComp->scale.y         = 1.0f;
    transformComp->facingDirection = facing;

    physicsComp->isStaticBody = false;
    physicsComp->size.x       = static_cast<float>(chrConfig->size.x);
    physicsComp->size.y       = static_cast<float>(chrConfig->size.y);
    physicsComp->position.x   = x;
    physicsComp->position.y   = y;
    physicsComp->position.z   = 0.0f;
    physicsComp->onGround     = 1;
    physicsComp->groundLevel  = 0.0f;
    physicsComp->lastPosition = physicsComp->position;

    identityComp->job = chrConfig->job;

    player->notifyEntityReady();
    return player;
}

int findBasicAttackSkillIndex(ActorDataComponent* actorData)
{
    for (size_t i = 0; i < actorData->skills.size(); ++i)
    {
        const auto& skill = actorData->skills[i];
        if (skill.skillData.fileName.find("basic_attack") != std::string::npos)
            return static_cast<int>(i);
        for (const auto& stage : skill.skillData.stages)
        {
            if (stage.animation.find("attack1") != std::string::npos)
                return static_cast<int>(i);
        }
    }
    return -1;
}

bool loadAnyMap(GameWord& world)
{
    for (int id = 1; id <= 32; ++id)
    {
        if (world.loadMap(id))
            return true;
    }
    return false;
}

FightResult runOnce(const std::string& contentRoot, uint64_t seed)
{
    FightResult result;
    io::clearSearchPaths();
    io::addSearchPath(contentRoot, true);
    if (!Config::getInstance()->loadConfig("mugen/config/config.bin"))
    {
        std::cerr << "loadConfig failed\n";
        return result;
    }

    GameWord world;
    if (!world.init(seed))
    {
        std::cerr << "GameWord::init failed\n";
        return result;
    }
    if (!loadAnyMap(world))
    {
        std::cerr << "loadMap failed\n";
        return result;
    }

    Entity* attacker = spawnSwordman(world, 200.0f, 100.0f, FacingDirection::kFacingRight);
    Entity* defender = spawnSwordman(world, 230.0f, 100.0f, FacingDirection::kFacingLeft);
    if (!attacker || !defender)
        return result;

    auto* actorData      = MG_GET_COMPONENT(attacker, ActorDataComponent);
    const int skillIndex = findBasicAttackSkillIndex(actorData);
    if (skillIndex < 0)
    {
        std::cerr << "basic_attack / attack1 skill not found, skillCount=" << actorData->skills.size() << "\n";
        for (size_t i = 0; i < actorData->skills.size(); ++i)
            std::cerr << "  skill[" << i << "]=" << actorData->skills[i].skillData.fileName << "\n";
        return result;
    }
    std::cerr << "using skillIndex=" << skillIndex << " file=" << actorData->skills[skillIndex].skillData.fileName
              << "\n";
    if (!actorData->skills[skillIndex].skillData.stages.empty())
    {
        std::cerr << "  stage0.animation=" << actorData->skills[skillIndex].skillData.stages[0].animation
                  << " hitType=" << static_cast<int>(actorData->skills[skillIndex].skillData.stages[0].hit.hitType)
                  << "\n";
    }

    auto* skillState                  = MG_GET_COMPONENT(attacker, SkillStateComponent);
    auto* fsmComp                     = MG_GET_COMPONENT(attacker, StatesMachineComponent);
    skillState->pendingSkillIndex     = skillIndex;
    if (!fsmComp->fsm.changeToStateByName("PlaySkill"))
    {
        std::cerr << "changeToState PlaySkill failed, cur=" << fsmComp->fsm.getCurStateName() << "\n";
        return result;
    }
    std::cerr << "attacker state=" << fsmComp->fsm.getCurStateName() << "\n";

    constexpr float dt      = 1.0f / 30.0f;
    constexpr int maxFrames = 180;
    for (int frame = 1; frame <= maxFrames; ++frame)
    {
        world.update(dt);

        auto* defFsm    = MG_GET_COMPONENT(defender, StatesMachineComponent);
        auto* atkSkill  = MG_GET_COMPONENT(attacker, SkillStateComponent);
        auto* atkAvatar = MG_GET_COMPONENT(attacker, AvatarComponent);
        auto* defAvatar = MG_GET_COMPONENT(defender, AvatarComponent);
        auto* atkFsm    = MG_GET_COMPONENT(attacker, StatesMachineComponent);

        if (frame <= 3 || (frame >= 6 && frame <= 10) || frame % 30 == 0)
        {
            int duration = -1;
            int timeMs   = -1;
            bool hasBox  = false;
            if (atkAvatar->playback)
            {
                duration = atkAvatar->playback->durationMs();
                timeMs   = atkAvatar->playback->currentTimeMs();
                hasBox   = atkAvatar->playback->combatTimeline() != nullptr;
            }
            std::cerr << "f" << frame << " atkState=" << atkFsm->fsm.getCurStateName()
                      << " motion=" << atkAvatar->animationName << " atkBoxes=" << atkAvatar->getAttackBoxes().size()
                      << " defBoxes=" << defAvatar->getDamageBoxes().size() << " t=" << timeMs << "/" << duration
                      << " hasBox=" << hasBox << " finished=" << atkAvatar->animationFinished << "\n";
        }

        if (!result.hit && atkSkill->hitTargets.count(defender->getId()))
        {
            result.hit      = true;
            result.hitFrame = static_cast<uint32_t>(frame);
            // Combat 在 StatesMachine 之后写入 pendingHits，受击状态下一帧才切换
            result.defenderStateAfterHit = std::string(defFsm->fsm.getCurStateName());
        }
        else if (result.hit && result.hitFrame + 1 == static_cast<uint32_t>(frame))
        {
            result.defenderStateAfterHit = std::string(defFsm->fsm.getCurStateName());
        }

        result.fingerprint += std::to_string(frame) + ":" + std::to_string(atkAvatar->getAttackBoxes().size()) + "/" +
                              std::to_string(defAvatar->getDamageBoxes().size()) + ":" +
                              std::string(defFsm->fsm.getCurStateName()) + ";";
    }

    return result;
}

bool prepareContentWithoutAni(const std::string& srcRoot, const std::string& dstRoot)
{
    const fs::path src = fs::path(srcRoot) / "mugen" / "config";
    const fs::path dst = fs::path(dstRoot) / "mugen" / "config";
    if (!fs::exists(src))
    {
        std::cerr << "source config missing: " << src.string() << "\n";
        return false;
    }

    std::error_code ec;
    fs::remove_all(dstRoot, ec);
    fs::create_directories(dst, ec);

    for (const auto& entry : fs::recursive_directory_iterator(src, ec))
    {
        if (!entry.is_regular_file())
            continue;
        const auto ext = entry.path().extension().string();
        if (ext == ".ani" || ext == ".ANI" || ext == ".png" || ext == ".PNG")
            continue;

        const auto rel = fs::relative(entry.path(), src, ec);
        if (ec)
            continue;
        const fs::path out = dst / rel;
        fs::create_directories(out.parent_path(), ec);
        fs::copy_file(entry.path(), out, fs::copy_options::overwrite_existing, ec);
        if (ec)
        {
            std::cerr << "copy failed: " << entry.path() << " -> " << out << " err=" << ec.message() << "\n";
            return false;
        }
    }
    return true;
}

/** C8: 服务器只认 .chr+.motion+.box，无 spine 运行时即可驱动判定 */
bool verifyNewheroLogicOnly(const std::string& contentRoot)
{
    io::clearSearchPaths();
    io::addSearchPath(contentRoot, true);
    if (!Config::getInstance()->loadConfig("mugen/config/config.bin"))
    {
        std::cerr << "newhero: loadConfig failed\n";
        return false;
    }

    const ChrConfig* chr = Config::getInstance()->getChrConfig("mugen/config/character/newhero/newhero.chr");
    if (!chr)
    {
        std::cerr << "newhero: ChrConfig load failed\n";
        return false;
    }
    if (chr->avatarType != AvatarType::kSpine)
    {
        std::cerr << "newhero: expected avatarType=Spine, got " << static_cast<int>(chr->avatarType) << "\n";
        return false;
    }

    auto* cache = AvatarAssetCache::getInstance();
    auto motionMap = cache->getMotionMap(chr->motionFile);
    if (!motionMap)
    {
        std::cerr << "newhero: MotionMap load failed: " << chr->motionFile << "\n";
        return false;
    }

    const MotionEntry* entry = motionMap->findEntry("attack1 motion", "attack1");
    if (!entry || entry->getType() != MotionEntryType::kSpine || entry->getSource().empty() ||
        entry->getBoxPath().empty())
    {
        std::cerr << "newhero: attack1 entry missing spine/box\n";
        return false;
    }

    MotionPlayback playback;
    playback.init(motionMap, "");
    playback.setAssetCache(cache);
    const bool loopFalse = false;
    if (!playback.play("attack1 motion", "attack1", &loopFalse))
    {
        std::cerr << "newhero: MotionPlayback::play failed\n";
        return false;
    }

    bool sawAttack = false;
    const int steps = playback.durationMs() / 16 + 2;
    for (int i = 0; i < steps; ++i)
    {
        playback.advance(16);
        std::vector<const DamageBox*> atk;
        std::vector<const DamageBox*> dmg;
        playback.boxesAt(atk, dmg);
        if (!atk.empty())
        {
            sawAttack = true;
            break;
        }
    }
    if (!sawAttack)
    {
        std::cerr << "newhero: no attack box during attack1 (duration=" << playback.durationMs() << ")\n";
        return false;
    }

    std::cout << "PASS: newhero spine logic (.chr+.motion+.box) attack box ok\n";
    return true;
}

}  // namespace

int main(int argc, char* argv[])
{
    std::string contentRoot = argc > 1 ? argv[1] : "../../../client/Content";
    if (!fs::exists(fs::path(contentRoot) / "mugen" / "config"))
    {
        const char* candidates[] = {
            "../../client/Content",
            "../../../../client/Content",
            "D:/work/AxmolFighter/client/Content",
        };
        for (const char* c : candidates)
        {
            if (fs::exists(fs::path(c) / "mugen" / "config"))
            {
                contentRoot = c;
                break;
            }
        }
    }

    contentRoot = fs::absolute(contentRoot).lexically_normal().string();
    std::cout << "content_root=" << contentRoot << "\n";

    constexpr uint64_t kSeed = 0xC601F001ULL;
    const FightResult a      = runOnce(contentRoot, kSeed);
    const FightResult b      = runOnce(contentRoot, kSeed);

    std::cout << "run1 hit=" << a.hit << " frame=" << a.hitFrame << " defState=" << a.defenderStateAfterHit << "\n";
    std::cout << "run2 hit=" << b.hit << " frame=" << b.hitFrame << " defState=" << b.defenderStateAfterHit << "\n";

    if (!(a.hit && b.hit && a.hitFrame == b.hitFrame && a.fingerprint == b.fingerprint))
    {
        std::cerr << "FAIL: determinism or hit check\n";
        return 1;
    }
    if (a.defenderStateAfterHit == "Idle" || a.defenderStateAfterHit == "PlaySkill" || a.defenderStateAfterHit.empty())
    {
        std::cerr << "FAIL: expected hit reaction state, got '" << a.defenderStateAfterHit << "'\n";
        return 1;
    }
    std::cout << "PASS: hit+deterministic frame=" << a.hitFrame << " reaction=" << a.defenderStateAfterHit << "\n";

    const std::string strippedRoot = (fs::temp_directory_path() / "axmol_battle_c6_no_ani").string();
    if (!prepareContentWithoutAni(contentRoot, strippedRoot))
    {
        std::cerr << "FAIL: prepare no-ani content\n";
        return 2;
    }

    const FightResult c = runOnce(strippedRoot, kSeed);
    std::cout << "no-ani hit=" << c.hit << " frame=" << c.hitFrame << " defState=" << c.defenderStateAfterHit << "\n";
    if (!c.hit || c.hitFrame != a.hitFrame || c.fingerprint != a.fingerprint)
    {
        std::cerr << "FAIL: no-ani run diverged (presentation leak)\n";
        return 3;
    }
    std::cout << "PASS: no-ani identical to with-ani\n";

    if (!verifyNewheroLogicOnly(contentRoot))
        return 4;

    return 0;
}
