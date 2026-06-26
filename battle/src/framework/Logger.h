#pragma once

#include <memory>

namespace spdlog
{
class logger;
}

namespace battle
{

std::shared_ptr<spdlog::logger> initLogger();
std::shared_ptr<spdlog::logger> logger();
void shutdownLogger();

}
