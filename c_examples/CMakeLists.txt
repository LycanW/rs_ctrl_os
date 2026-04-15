cmake_minimum_required(VERSION 3.22)

# Prefer g++ when unset: default `c++` may be clang without usable C++ stdlib headers.
if(UNIX AND NOT APPLE AND NOT DEFINED CMAKE_CXX_COMPILER)
    find_program(_RCOS_GXX NAMES g++)
    if(_RCOS_GXX)
        set(CMAKE_CXX_COMPILER "${_RCOS_GXX}" CACHE FILEPATH "C++ compiler for rcos_minimal" FORCE)
    endif()
endif()

project(rcos_minimal LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# Parent repo root (contains include/rs_ctrl_os.h and target/.../librs_ctrl_os.a when built from source)
set(RCOS_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/.." CACHE PATH "rs_ctrl_os checkout or prefix with include/ and lib/")

set(RCOS_LIB "" CACHE FILEPATH "Path to librs_ctrl_os.a (empty = auto from RCOS_ROOT)")
if(NOT RCOS_LIB)
    set(_default_lib "${RCOS_ROOT}/target/release/librs_ctrl_os.a")
    if(EXISTS "${_default_lib}")
        set(RCOS_LIB "${_default_lib}")
    else()
        set(_debug_lib "${RCOS_ROOT}/target/debug/librs_ctrl_os.a")
        if(EXISTS "${_debug_lib}")
            set(RCOS_LIB "${_debug_lib}")
        endif()
    endif()
endif()
if(NOT RCOS_LIB OR NOT EXISTS "${RCOS_LIB}")
    message(FATAL_ERROR "Set RCOS_LIB to librs_ctrl_os.a (e.g. build with: cargo build --release in RCOS_ROOT). Current: '${RCOS_LIB}'")
endif()

add_executable(rcos_minimal minimal.cpp)
target_compile_features(rcos_minimal PRIVATE cxx_std_23)
target_include_directories(rcos_minimal PRIVATE "${RCOS_ROOT}/include")

find_package(PkgConfig QUIET)
if(PkgConfig_FOUND)
    pkg_check_modules(ZMQ IMPORTED_TARGET libzmq)
endif()

if(TARGET PkgConfig::ZMQ)
    target_link_libraries(rcos_minimal PRIVATE "${RCOS_LIB}" PkgConfig::ZMQ)
else()
    find_library(ZMQ_LIBRARY NAMES zmq REQUIRED)
    target_link_libraries(rcos_minimal PRIVATE "${RCOS_LIB}" "${ZMQ_LIBRARY}")
endif()

if(UNIX AND NOT APPLE)
    target_link_libraries(rcos_minimal PRIVATE pthread dl m)
endif()
