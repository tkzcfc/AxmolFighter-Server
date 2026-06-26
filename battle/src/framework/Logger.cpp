#include "framework/Logger.h"

#include <filesystem>
#include <memory>
#include <spdlog/sinks/rotating_file_sink.h>
#include <spdlog/sinks/stdout_color_sinks.h>
#include <spdlog/spdlog.h>
#include <vector>

namespace battle
{
namespace
{
std::shared_ptr<spdlog::logger> g_logger;
}

std::shared_ptr<spdlog::logger> initLogger()
{
    if (g_logger)
        return g_logger;

    std::filesystem::create_directories("logs");

    auto consoleSink = std::make_shared<spdlog::sinks::stdout_color_sink_mt>();
    auto fileSink = std::make_shared<spdlog::sinks::rotating_file_sink_mt>(
        "logs/battle_server.log", 10 * 1024 * 1024, 5);

    std::vector<spdlog::sink_ptr> sinks{consoleSink, fileSink};
    g_logger = std::make_shared<spdlog::logger>("battle", sinks.begin(), sinks.end());
    g_logger->set_level(spdlog::level::info);
    g_logger->set_pattern("[%Y-%m-%d %H:%M:%S.%e] [%l] [%n] %v");
    g_logger->flush_on(spdlog::level::warn);

    spdlog::set_default_logger(g_logger);
    return g_logger;
}

std::shared_ptr<spdlog::logger> logger()
{
    return initLogger();
}

void shutdownLogger()
{
    if (g_logger)
        g_logger->flush();
    g_logger.reset();
    spdlog::shutdown();
}

}
