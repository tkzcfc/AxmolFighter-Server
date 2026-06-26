#include "framework/BackendCodec.h"

#include <cstring>

namespace battle
{

std::uint16_t readU16BE(const char* data)
{
    return (static_cast<std::uint16_t>(static_cast<unsigned char>(data[0])) << 8) |
        static_cast<std::uint16_t>(static_cast<unsigned char>(data[1]));
}

std::uint32_t readU32BE(const char* data)
{
    return (static_cast<std::uint32_t>(static_cast<unsigned char>(data[0])) << 24) |
        (static_cast<std::uint32_t>(static_cast<unsigned char>(data[1])) << 16) |
        (static_cast<std::uint32_t>(static_cast<unsigned char>(data[2])) << 8) |
        static_cast<std::uint32_t>(static_cast<unsigned char>(data[3]));
}

std::int32_t readI32BE(const char* data)
{
    return static_cast<std::int32_t>(readU32BE(data));
}

void writeU16BE(std::string& data, std::size_t offset, std::uint16_t value)
{
    data[offset] = static_cast<char>((value >> 8) & 0xFF);
    data[offset + 1] = static_cast<char>(value & 0xFF);
}

void writeU32BE(std::string& data, std::size_t offset, std::uint32_t value)
{
    data[offset] = static_cast<char>((value >> 24) & 0xFF);
    data[offset + 1] = static_cast<char>((value >> 16) & 0xFF);
    data[offset + 2] = static_cast<char>((value >> 8) & 0xFF);
    data[offset + 3] = static_cast<char>(value & 0xFF);
}

void writeI32BE(std::string& data, std::size_t offset, std::int32_t value)
{
    writeU32BE(data, offset, static_cast<std::uint32_t>(value));
}

std::string encodeBackendFrame(std::uint8_t cmd,
                               std::uint16_t msgId,
                               std::int32_t serial,
                               std::uint32_t sessionId,
                               const char* payload,
                               std::size_t payloadLen)
{
    const auto frameLen = static_cast<std::uint32_t>(kBackendFrameHeaderSize + payloadLen);
    std::string frame(frameLen, '\0');
    writeU32BE(frame, 0, frameLen);
    frame[4] = static_cast<char>(cmd);
    writeU16BE(frame, 5, msgId);
    writeI32BE(frame, 7, serial);
    writeU32BE(frame, 11, sessionId);
    if (payloadLen > 0)
        std::memcpy(frame.data() + kBackendFrameHeaderSize, payload, payloadLen);
    return frame;
}

bool decodeBackendFrameBody(std::string_view data, BackendFrame& frame, std::string* error)
{
    if (data.size() < kBackendFrameBodyHeaderSize)
    {
        if (error)
            *error = "backend frame body too short";
        return false;
    }

    frame.cmd = static_cast<std::uint8_t>(data[0]);
    frame.msgId = readU16BE(data.data() + 1);
    frame.serial = readI32BE(data.data() + 3);
    frame.sessionId = readU32BE(data.data() + 7);
    frame.payload = std::string_view(data.data() + kBackendFrameBodyHeaderSize,
                                     data.size() - kBackendFrameBodyHeaderSize);
    return true;
}

bool decodeBackendFrame(std::string_view data, BackendFrame& frame, std::string* error)
{
    if (data.size() < kBackendFrameHeaderSize)
    {
        if (error)
            *error = "backend frame too short";
        return false;
    }

    const std::uint32_t frameLen = readU32BE(data.data());
    if (frameLen < kBackendFrameHeaderSize)
    {
        if (error)
            *error = "invalid backend frame length";
        return false;
    }
    if (frameLen > kMaxBackendPacketSize)
    {
        if (error)
            *error = "backend frame too large";
        return false;
    }
    if (frameLen > data.size())
    {
        if (error)
            *error = "incomplete backend frame";
        return false;
    }

    return decodeBackendFrameBody(
        std::string_view(data.data() + 4, frameLen - 4), frame, error);
}

}
