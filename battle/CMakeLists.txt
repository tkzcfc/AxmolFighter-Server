cmake_minimum_required(VERSION 3.16)
project(battle_server C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

set(YASIO_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../client/axmol/3rdparty/yasio")
set(CLIENT_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../client/Source")
set(PROTOBUF_DIR "${CLIENT_SOURCE_DIR}/3rd/protobuf")
set(MUGEN_DIR "${CLIENT_SOURCE_DIR}/mugen")

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

# ========== protobuf-lite ==========
set(protobuf_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(protobuf_BUILD_PROTOC_BINARIES OFF CACHE BOOL "" FORCE)
set(protobuf_BUILD_SHARED_LIBS OFF CACHE BOOL "" FORCE)
set(protobuf_WITH_ZLIB OFF CACHE BOOL "" FORCE)
set(protobuf_MSVC_STATIC_RUNTIME OFF CACHE BOOL "" FORCE)
set(protobuf_BUILD_LIBPROTOC OFF CACHE BOOL "" FORCE)
add_subdirectory("${PROTOBUF_DIR}/cmake" protobuf EXCLUDE_FROM_ALL)
if(WIN32)
    set_property(TARGET libprotobuf-lite PROPERTY INTERFACE_LINK_LIBRARIES "")
    if(TARGET libprotobuf)
        set_property(TARGET libprotobuf PROPERTY INTERFACE_LINK_LIBRARIES "")
    endif()
endif()

# ========== spdlog ==========
find_package(spdlog CONFIG QUIET)
if(NOT spdlog_FOUND)
    FetchContent_Declare(
        spdlog
        GIT_REPOSITORY https://github.com/gabime/spdlog.git
        GIT_TAG v1.14.1
    )
    FetchContent_MakeAvailable(spdlog)
endif()
add_compile_definitions(FMT_USE_CONSTEVAL=0)
if(TARGET spdlog)
    target_compile_definitions(spdlog PUBLIC FMT_USE_CONSTEVAL=0 FMT_CONSTEVAL=)
endif()

# ========== mugen headless ==========
file(GLOB_RECURSE MUGEN_SOURCES
    "${MUGEN_DIR}/*.cpp"
    "${MUGEN_DIR}/*.h"
)

list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/render/.*")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/resource/Render.*")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/resource/ResourceManager\\.(cpp|h)$")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/resource/RenderObjectPool\\.(cpp|h)$")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/system/AvatarRenderSystem\\.(cpp|h)$")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/system/GameMapRenderSystem\\.(cpp|h)$")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/avatar/Avatar\\.(cpp|h)$")
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/avatar/AnimationPlayer\\.(cpp|h)$")

add_library(mugen_headless STATIC ${MUGEN_SOURCES})
target_sources(mugen_headless PRIVATE
    "${CLIENT_SOURCE_DIR}/3rd/tinyexpr/tinyexpr.c"
)
target_compile_definitions(mugen_headless PUBLIC RUNTIME_IN_AXMOL=0)
target_include_directories(mugen_headless PUBLIC
    "${CLIENT_SOURCE_DIR}"
    "${CLIENT_SOURCE_DIR}/3rd/tinyexpr"
)
if(MSVC)
    target_compile_options(mugen_headless PRIVATE /utf-8)
endif()

# ========== battle_server ==========
file(GLOB_RECURSE BATTLE_SOURCES
    "src/*.cpp"
    "src/*.h"
)

add_executable(${PROJECT_NAME} ${BATTLE_SOURCES})
target_include_directories(${PROJECT_NAME} PRIVATE
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    "${YASIO_DIR}"
    "${CLIENT_SOURCE_DIR}"
    "${CLIENT_SOURCE_DIR}/net"
    "${PROTOBUF_DIR}/src"
)
target_sources(${PROJECT_NAME} PRIVATE
    "${CLIENT_SOURCE_DIR}/net/game.pb.cc"
    "${CLIENT_SOURCE_DIR}/net/gateway.pb.cc"
)
target_link_libraries(${PROJECT_NAME} PRIVATE
    yasio
    mugen_headless
    libprotobuf-lite
    spdlog::spdlog
)

if(WIN32)
    target_link_libraries(${PROJECT_NAME} PRIVATE ws2_32)
endif()

if(MSVC)
    target_compile_options(${PROJECT_NAME} PRIVATE /utf-8)
endif()

# 输出到 bin 目录
set_target_properties(${PROJECT_NAME} PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/bin"
)
