#pragma once

#include "framework/Protocol.h"
#include <cstdint>
#include <string>
#include <string_view>

namespace battle
{

struct BackendFrame
{
    std::uint8_t cmd = 0;
    std::uint16_t msgId = 0;
    std::int32_t serial = 0;
    std::uint32_t sessionId = 0;
    std::string_view payload;
};

template <typename PbMessage>
bool parsePayload(PbMessage& message, std::string_view payload)
{
    return message.ParseFromArray(payload.data(), static_cast<int>(payload.size()));
}

std::uint16_t readU16BE(const char* data);
std::uint32_t readU32BE(const char* data);
std::int32_t readI32BE(const char* data);

void writeU16BE(std::string& data, std::size_t offset, std::uint16_t value);
void writeU32BE(std::string& data, std::size_t offset, std::uint32_t value);
void writeI32BE(std::string& data, std::size_t offset, std::int32_t value);

std::string encodeBackendFrame(std::uint8_t cmd,
                               std::uint16_t msgId,
                               std::int32_t serial,
                               std::uint32_t sessionId,
                               const char* payload,
                               std::size_t payloadLen);

bool decodeBackendFrame(std::string_view data, BackendFrame& frame, std::string* error = nullptr);
bool decodeBackendFrameBody(std::string_view data, BackendFrame& frame, std::string* error = nullptr);

}
