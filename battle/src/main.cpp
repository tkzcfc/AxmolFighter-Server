#include "BattleServer.h"
#include "BattleConfigLoader.h"
#include "framework/Logger.h"

#include <atomic>
#include <chrono>
#include <csignal>
#include <spdlog/spdlog.h>
#include <string>
#include <thread>

static BattleServer* g_server = nullptr;
static std::atomic_bool g_exiting = false;

void signalHandler(int sig)
{
    spdlog::info("Received signal {}, shutting down", sig);
    g_exiting = true;
    if (g_server)
        g_server->shutdown();
}

static void sleepBeforeRestart(float seconds)
{
    if (seconds <= 0.0f)
        return;

    const auto deadline = std::chrono::steady_clock::now() +
        std::chrono::duration_cast<std::chrono::steady_clock::duration>(
            std::chrono::duration<float>(seconds));

    while (!g_exiting && std::chrono::steady_clock::now() < deadline)
        std::this_thread::sleep_for(std::chrono::milliseconds(100));
}

int main(int argc, char* argv[])
{
    battle::initLogger();

    std::string configPath = "config/battle.toml";
    if (argc > 1)
        configPath = argv[1];

    const auto config = loadBattleServerConfig(configPath);

    spdlog::info("Battle Server starting instance_id={} gateway={}:{} tick_rate={} max_battles={} max_sessions={} restart_on_gateway_disconnect={}",
                 config.instanceId,
                 config.gatewayHost,
                 config.gatewayPort,
                 config.tickRate,
                 config.maxBattles,
                 config.maxSessions,
                 config.restartOnGatewayDisconnect);

    std::signal(SIGINT, signalHandler);
    std::signal(SIGTERM, signalHandler);

    int exitCode = 0;
    while (!g_exiting)
    {
        bool initOk = false;
        {
            BattleServer server;
            g_server = &server;

            initOk = server.init(config);
            if (!initOk)
            {
                spdlog::error("Battle Server init failed");
            }
            else
            {
                server.run();
            }

            g_server = nullptr;
        }

        if (g_exiting)
            break;

        if (!config.restartOnGatewayDisconnect)
        {
            spdlog::error("{}; exiting because restart_on_gateway_disconnect=false",
                          initOk ? "Battle Server stopped after gateway disconnect/failure"
                                 : "Battle Server init failed");
            exitCode = 1;
            break;
        }

        spdlog::warn("{}; restarting in {:.2f}s",
                     initOk ? "Battle Server stopped after gateway disconnect/failure"
                            : "Battle Server init failed",
                     config.reconnectInterval);
        sleepBeforeRestart(config.reconnectInterval);
    }

    g_server = nullptr;
    battle::shutdownLogger();
    return exitCode;
}
