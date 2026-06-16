cmake_minimum_required(VERSION 3.16)
project(battle_server CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# ========== 客户端源码路径 ==========
set(CLIENT_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../client/Source")
set(YASIO_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../client/axmol/3rdparty/yasio")

if(NOT EXISTS "${YASIO_DIR}/yasio/yasio.hpp")
    message(FATAL_ERROR "yasio not found at: ${YASIO_DIR}")
endif()

# ========== yasio ==========
add_library(yasio STATIC
    "${YASIO_DIR}/yasio/io_service.cpp"
    "${YASIO_DIR}/yasio/xxsocket.cpp"
)
target_include_directories(yasio PUBLIC "${YASIO_DIR}")

if(WIN32)
    target_link_libraries(yasio PUBLIC ws2_32)
endif()

# ========== 共享网络层（来自客户端） ==========
set(SHARED_NET_SOURCES
    "${CLIENT_SOURCE_DIR}/net/YasioClient.h"
    "${CLIENT_SOURCE_DIR}/net/YasioClient.cpp"
    "${CLIENT_SOURCE_DIR}/net/NetClient.h"
    "${CLIENT_SOURCE_DIR}/net/NetClient.cpp"
)

# ========== battle_server ==========
file(GLOB_RECURSE BATTLE_SOURCES
    "src/*.cpp"
    "src/*.h"
)

add_executable(${PROJECT_NAME} ${BATTLE_SOURCES} ${SHARED_NET_SOURCES})
target_include_directories(${PROJECT_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    "${CLIENT_SOURCE_DIR}/net"
)
target_link_libraries(${PROJECT_NAME} PRIVATE yasio)

if(WIN32)
    target_link_libraries(${PROJECT_NAME} PRIVATE ws2_32)
endif()

# 输出到 bin 目录
set_target_properties(${PROJECT_NAME} PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/bin"
)
