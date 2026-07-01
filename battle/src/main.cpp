#include "BattleServer.h"
#include "BattleConfigLoader.h"
#include "framework/Logger.h"

#include <atomic>
#include <csignal>
#include <spdlog/spdlog.h>
#include <string>

static BattleServer* g_server = nullptr;
static std::atomic_bool g_exiting = false;

void signalHandler(int sig)
{
    spdlog::info("Received signal {}, shutting down", sig);
    g_exiting = true;
    if (g_server)
        g_server->shutdown();
}

int main(int argc, char* argv[])
{
    battle::initLogger();

    std::string configPath = "config/battle.toml";
    if (argc > 1)
        configPath = argv[1];

    const auto config = loadBattleServerConfig(configPath);

    spdlog::info("Battle Server starting instance_id={} gateway={}:{} tick_rate={} max_battles={} max_sessions={}",
                 config.instanceId,
                 config.gatewayHost,
                 config.gatewayPort,
                 config.tickRate,
                 config.maxBattles,
                 config.maxSessions);

    std::signal(SIGINT, signalHandler);
    std::signal(SIGTERM, signalHandler);

    int exitCode = 0;
    BattleServer server;
    g_server = &server;

    if (!server.init(config))
    {
        spdlog::error("Battle Server init failed");
        exitCode = 1;
    }
    else
    {
        server.run();
        if (!g_exiting)
        {
            spdlog::warn("Battle Server stopped after gateway disconnect/failure");
            exitCode = 1;
        }
    }

    g_server = nullptr;
    battle::shutdownLogger();
    return exitCode;
}
