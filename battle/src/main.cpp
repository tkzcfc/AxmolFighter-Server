#include "BattleServer.h"
#include "framework/Logger.h"

#include <csignal>
#include <cstdlib>
#include <fstream>
#include <spdlog/spdlog.h>
#include <string>

static BattleServer* g_server = nullptr;

void signalHandler(int sig)
{
    spdlog::info("Received signal {}, shutting down", sig);
    if (g_server)
        g_server->shutdown();
}

static std::string trim(const std::string& s)
{
    const auto start = s.find_first_not_of(" \t\r\n");
    if (start == std::string::npos)
        return "";
    const auto end = s.find_last_not_of(" \t\r\n");
    return s.substr(start, end - start + 1);
}

static std::string stripQuotes(const std::string& s)
{
    if (s.size() >= 2 && s.front() == '"' && s.back() == '"')
        return s.substr(1, s.size() - 2);
    return s;
}

static BattleServerConfig loadConfig(const std::string& path)
{
    BattleServerConfig config;
    std::ifstream file(path);
    if (!file.is_open())
    {
        spdlog::warn("Config file not found: {}, using defaults", path);
        return config;
    }

    std::string section;
    std::string line;
    while (std::getline(file, line))
    {
        line = trim(line);
        if (line.empty() || line[0] == '#')
            continue;

        if (line.front() == '[' && line.back() == ']')
        {
            section = line.substr(1, line.size() - 2);
            continue;
        }

        const auto eq = line.find('=');
        if (eq == std::string::npos)
            continue;

        const auto key = trim(line.substr(0, eq));
        const auto value = trim(line.substr(eq + 1));

        if (section == "server")
        {
            if (key == "instance_id")
                config.instanceId = static_cast<std::uint32_t>(std::stoul(value));
            else if (key == "tick_rate")
                config.tickRate = std::stoi(value);
            else if (key == "max_battles")
                config.maxBattles = static_cast<std::uint32_t>(std::stoul(value));
            else if (key == "max_sessions")
                config.maxSessions = static_cast<std::uint32_t>(std::stoul(value));
            else if (key == "load_report_interval")
                config.loadReportInterval = std::stof(value);
        }
        else if (section == "gateway")
        {
            if (key == "host")
                config.gatewayHost = stripQuotes(value);
            else if (key == "port")
                config.gatewayPort = std::stoi(value);
            else if (key == "reconnect_interval")
                config.reconnectInterval = std::stof(value);
        }
    }

    return config;
}

int main(int argc, char* argv[])
{
    battle::initLogger();

    std::string configPath = "config/battle.toml";
    if (argc > 1)
        configPath = argv[1];

    const auto config = loadConfig(configPath);

    spdlog::info("Battle Server starting instance_id={} gateway={}:{} tick_rate={} max_battles={} max_sessions={}",
                 config.instanceId,
                 config.gatewayHost,
                 config.gatewayPort,
                 config.tickRate,
                 config.maxBattles,
                 config.maxSessions);

    BattleServer server;
    g_server = &server;

    std::signal(SIGINT, signalHandler);
    std::signal(SIGTERM, signalHandler);

    if (!server.init(config))
    {
        spdlog::error("Battle Server init failed");
        battle::shutdownLogger();
        return 1;
    }

    server.run();

    g_server = nullptr;
    battle::shutdownLogger();
    return 0;
}
