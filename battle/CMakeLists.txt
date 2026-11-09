cmake_minimum_required(VERSION 3.16)
project(battle_server C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

set(YASIO_DIR "${CMAKE_CURRENT_SOURCE_DIR}/3rd/yasio")
set(CLIENT_SOURCE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../AxmolFighter-Client/Source")
set(AXMOL_3RDPARTY_DIR "${CMAKE_CURRENT_SOURCE_DIR}/3rd")
set(PROTOBUF_DIR "${CLIENT_SOURCE_DIR}/3rd/protobuf")
set(MUGEN_DIR "${CLIENT_SOURCE_DIR}/mugen")

file(GLOB CLIENT_PROTO_SOURCES CONFIGURE_DEPENDS
    "${CLIENT_SOURCE_DIR}/net/*.pb.cc"
)
list(FILTER CLIENT_PROTO_SOURCES EXCLUDE REGEX ".*/game_(battle|town)\\.pb\\.cc$")

file(GLOB BATTLE_PROTO_SOURCES CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/src/protocol/*.pb.cc"
)

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

# ========== mugen (shared battle core, mirrors TestsHeadless source set) ==========
function(mugen_collect_sources out_var)
    set(collected)
    foreach(pattern IN LISTS ARGN)
        file(GLOB_RECURSE matched CONFIGURE_DEPENDS ${pattern})
        list(APPEND collected ${matched})
    endforeach()
    set(${out_var} ${collected} PARENT_SCOPE)
endfunction()

mugen_collect_sources(MUGEN_SOURCES
    "${MUGEN_DIR}/core/*.h"
    "${MUGEN_DIR}/core/*.hpp"
    "${MUGEN_DIR}/core/*.cpp"
    "${MUGEN_DIR}/core/*.c"
    "${MUGEN_DIR}/conf/*.h"
    "${MUGEN_DIR}/conf/*.hpp"
    "${MUGEN_DIR}/conf/*.cpp"
    "${MUGEN_DIR}/conf/*.c"
    "${MUGEN_DIR}/avatar/*.h"
    "${MUGEN_DIR}/avatar/*.hpp"
    "${MUGEN_DIR}/avatar/*.cpp"
    "${MUGEN_DIR}/avatar/*.c"
    "${MUGEN_DIR}/sim_legacy/*.h"
    "${MUGEN_DIR}/sim_legacy/*.hpp"
    "${MUGEN_DIR}/sim_legacy/*.cpp"
    "${MUGEN_DIR}/sim_legacy/*.c"
)
# Rendering stays on the client; excludes both mugen/render and mugen/avatar/render.
list(FILTER MUGEN_SOURCES EXCLUDE REGEX ".*/render/.*")

include("${CLIENT_SOURCE_DIR}/../cmake/VfsSources.cmake")
list(APPEND MUGEN_SOURCES ${VFS_SOURCES})

add_library(mugen STATIC ${MUGEN_SOURCES})
target_compile_definitions(mugen PUBLIC RUNTIME_IN_AXMOL=0)
target_include_directories(mugen PUBLIC
    "${CLIENT_SOURCE_DIR}"
    "${CLIENT_SOURCE_DIR}/3rd"
    "${AXMOL_3RDPARTY_DIR}"
)
if(MSVC)
    target_compile_options(mugen PRIVATE /utf-8)
endif()
find_package(Threads REQUIRED)
target_link_libraries(mugen PUBLIC Threads::Threads)

# ========== battle_core (room logic + content mount + logger, no yasio) ==========
file(GLOB_RECURSE BATTLE_CORE_SOURCES CONFIGURE_DEPENDS
    "src/battle/*.cpp"
    "src/battle/*.h"
    "src/framework/Logger.cpp"
    "src/framework/Logger.h"
)

add_library(battle_core STATIC ${BATTLE_CORE_SOURCES})
target_include_directories(battle_core PUBLIC
    "${CMAKE_CURRENT_SOURCE_DIR}/src"
    "${CMAKE_CURRENT_SOURCE_DIR}/src/protocol"
    "${CLIENT_SOURCE_DIR}"
    "${CLIENT_SOURCE_DIR}/net"
    "${PROTOBUF_DIR}/src"
)
target_link_libraries(battle_core PUBLIC
    mugen
    libprotobuf-lite
    spdlog::spdlog
)
if(MSVC)
    target_compile_options(battle_core PRIVATE /utf-8)
endif()

# ========== battle_server ==========
file(GLOB_RECURSE BATTLE_SERVER_SOURCES CONFIGURE_DEPENDS
    "src/main.cpp"
    "src/BattleServer.cpp"
    "src/BattleServer.h"
    "src/BattleConfigLoader.cpp"
    "src/BattleConfigLoader.h"
    "src/framework/BackendClient.cpp"
    "src/framework/BackendClient.h"
    "src/framework/BackendCodec.cpp"
    "src/framework/BackendCodec.h"
    "src/framework/RpcManager.cpp"
    "src/framework/RpcManager.h"
)

add_executable(${PROJECT_NAME} ${BATTLE_SERVER_SOURCES})
target_sources(${PROJECT_NAME} PRIVATE
    ${CLIENT_PROTO_SOURCES}
    ${BATTLE_PROTO_SOURCES}
)
target_link_libraries(${PROJECT_NAME} PRIVATE
    battle_core
    yasio
)
if(WIN32)
    target_link_libraries(${PROJECT_NAME} PRIVATE ws2_32)
endif()
if(MSVC)
    target_compile_options(${PROJECT_NAME} PRIVATE /utf-8)
endif()

set_target_properties(${PROJECT_NAME} PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/bin"
)

# ========== battle_tests (headless room logic tests, no network) ==========
file(GLOB_RECURSE BATTLE_TEST_SOURCES CONFIGURE_DEPENDS
    "tests/*.cpp"
    "tests/*.h"
)

if(BATTLE_TEST_SOURCES)
    add_executable(battle_tests ${BATTLE_TEST_SOURCES})
    target_sources(battle_tests PRIVATE
        ${CLIENT_PROTO_SOURCES}
        ${BATTLE_PROTO_SOURCES}
    )
    target_link_libraries(battle_tests PRIVATE battle_core)
    target_compile_definitions(battle_tests PRIVATE
        MUGEN_CONTENT_DIR="${CLIENT_SOURCE_DIR}/../Content"
    )
    if(MSVC)
        target_compile_options(battle_tests PRIVATE /utf-8)
    endif()

    set_target_properties(battle_tests PROPERTIES
        RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/bin"
    )
endif()
