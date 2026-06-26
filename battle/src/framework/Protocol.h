#pragma once

#include <cstddef>
#include <cstdint>

namespace battle
{

inline constexpr std::size_t kBackendFrameHeaderSize = 15;
inline constexpr std::size_t kBackendFrameBodyHeaderSize = 11;
inline constexpr std::uint32_t kMaxBackendPacketSize = 1024 * 1024;

inline constexpr std::uint8_t kCmdBusiness = 1;
inline constexpr std::uint8_t kCmdGatewayControl = 2;

inline constexpr std::uint32_t kServiceIdGame = 0;
inline constexpr std::uint32_t kServiceIdBattle = 1;
inline constexpr std::uint32_t kServiceIdTown = 2;

inline constexpr std::int32_t kRegisterSerial = -1;
inline constexpr float kDefaultRpcTimeoutSeconds = 10.0f;

}
