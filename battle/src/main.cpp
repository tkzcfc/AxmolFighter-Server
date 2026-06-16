#include "BattleServer.h"
#include <cstdio>
#include <cstdlib>
#include <csignal>
#include <fstream>
#include <sstream>
#include <string>

static BattleServer* g_server = nullptr;

void signalHandler(int sig)
{
    printf("\n[main] received signal %d, shutting down...\n", sig);
    if (g_server)
        g_server->shutdown();
}

/// 简单的 TOML 解析（只支持基本 key=value）
static std::string trim(const std::string& s)
{
    auto start = s.find_first_not_of(" \t\r\n");
    if (start == std::string::npos)
        return "";
    auto end = s.find_last_not_of(" \t\r\n");
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
        printf("[main] config file not found: %s, using defaults\n", path.c_str());
        return config;
    }

    std::string line;
    while (std::getline(file, line))
    {
        line = trim(line);
        if (line.empty() || line[0] == '#' || line[0] == '[')
            continue;

        auto eq = line.find('=');
        if (eq == std::string::npos)
            continue;

        auto key   = trim(line.substr(0, eq));
        auto value = trim(line.substr(eq + 1));

        if (key == "instance_id")
            config.instanceId = static_cast<uint32_t>(std::stoul(value));
        else if (key == "tick_rate")
            config.tickRate = std::stoi(value);
        else if (key == "host")
            config.gatewayHost = stripQuotes(value);
        else if (key == "port")
            config.gatewayPort = std::stoi(value);
        else if (key == "reconnect_interval")
            config.reconnectInterval = std::stof(value);
    }

    return config;
}

int main(int argc, char* argv[])
{
    // 默认配置路径
    std::string configPath = "config/battle.toml";
    if (argc > 1)
        configPath = argv[1];

    auto config = loadConfig(configPath);

    printf("[main] Battle Server starting (instance_id=%u, gateway=%s:%d, tick_rate=%d)\n",
           config.instanceId, config.gatewayHost.c_str(), config.gatewayPort, config.tickRate);

    BattleServer server;
    g_server = &server;

    signal(SIGINT, signalHandler);
    signal(SIGTERM, signalHandler);

    if (!server.init(config))
    {
        printf("[main] init failed\n");
        return 1;
    }

    server.run();

    g_server = nullptr;
    return 0;
}
