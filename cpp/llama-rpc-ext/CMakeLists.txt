cmake_minimum_required(VERSION 3.20)
project(llama_rpc_ext LANGUAGES CXX)

# Backend detection — driven by the build host:
#   - Apple Silicon  → BUILD_BACKEND_METAL  ON
#   - Linux+CUDA     → BUILD_BACKEND_CUDA   ON
#   - other          → CPU fallback only (useful for CI / dry runs)
#
# We keep the build single-target so a Mac worker pkg ships only the Metal
# server, and a Linux/CUDA worker container ships only the CUDA server.

option(BUILD_RPC_SERVER       "Build cluster RPC server"           OFF)
option(BUILD_BACKEND_CUDA     "Link CUDA backend (Linux/Win)"      OFF)
option(BUILD_BACKEND_METAL    "Link Metal backend (macOS only)"    OFF)

if(APPLE AND NOT DEFINED BUILD_BACKEND_METAL_DEFAULTED)
    # On macOS default Metal ON, CUDA OFF.
    set(BUILD_BACKEND_METAL ON CACHE BOOL "" FORCE)
    set(BUILD_BACKEND_CUDA  OFF CACHE BOOL "" FORCE)
    set(BUILD_BACKEND_METAL_DEFAULTED ON CACHE INTERNAL "")
endif()

if(BUILD_BACKEND_CUDA)
    enable_language(CUDA)
    set(CMAKE_CUDA_STANDARD 17)
    # Multi-arch fat binary (Ampere/Ada/Hopper/Blackwell)
    set(CMAKE_CUDA_ARCHITECTURES 86 89 90 120)
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Phase 2: this directory will host a Git submodule clone of llama.cpp
# plus our custom hooks for cluster-aware RPC.
#
# Targets:
#   - rpc-server-ext   : extended ggml RPC server with cluster auth + metrics
#   - llama-rpc-glue   : static lib linked into the worker

if(BUILD_RPC_SERVER)
    set(SUBMODULE_DIR ${CMAKE_CURRENT_SOURCE_DIR}/third_party/llama.cpp)
    if(EXISTS ${SUBMODULE_DIR}/CMakeLists.txt)
        # Forward backend selection into upstream llama.cpp.
        if(BUILD_BACKEND_METAL)
            set(GGML_METAL ON  CACHE BOOL "" FORCE)
            set(GGML_CUDA  OFF CACHE BOOL "" FORCE)
        endif()
        if(BUILD_BACKEND_CUDA)
            set(GGML_CUDA  ON  CACHE BOOL "" FORCE)
            set(GGML_METAL OFF CACHE BOOL "" FORCE)
        endif()
        # ggml-rpc is built as part of upstream when GGML_RPC=ON.
        set(GGML_RPC ON CACHE BOOL "" FORCE)
        # Force upstream to also build its own `rpc-server` (tools/rpc) which we
        # use as the bringup binary for Phase 2. Our `rpc-server-ext` is a
        # cluster-aware wrapper that wraps/extends it.
        set(LLAMA_BUILD_TOOLS ON CACHE BOOL "" FORCE)
        set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
        set(LLAMA_BUILD_TESTS OFF CACHE BOOL "" FORCE)
        add_subdirectory(${SUBMODULE_DIR} llama_cpp_build)
    endif()

    add_executable(rpc-server-ext src/main.cpp)
    target_compile_features(rpc-server-ext PRIVATE cxx_std_20)

    if(BUILD_BACKEND_METAL)
        target_compile_definitions(rpc-server-ext PRIVATE GPUCLUSTER_BACKEND_METAL=1)
        # Apple frameworks needed by the Metal backend.
        find_library(METAL_FRAMEWORK Metal)
        find_library(FOUNDATION_FRAMEWORK Foundation)
        find_library(METALKIT_FRAMEWORK MetalKit)
        target_link_libraries(rpc-server-ext PRIVATE
            ${METAL_FRAMEWORK} ${FOUNDATION_FRAMEWORK} ${METALKIT_FRAMEWORK})
    endif()
    if(BUILD_BACKEND_CUDA)
        target_compile_definitions(rpc-server-ext PRIVATE GPUCLUSTER_BACKEND_CUDA=1)
    endif()
endif()
