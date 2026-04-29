cmake_minimum_required(VERSION 3.20)
project(llama_rpc_ext LANGUAGES CXX CUDA)

# Phase 2: this directory will host a Git submodule clone of llama.cpp
# plus our custom hooks for cluster-aware RPC.
#
# Targets:
#   - rpc-server-ext   : extended ggml RPC server with cluster auth + metrics
#   - llama-rpc-glue   : static lib linked into the worker

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CUDA_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Multi-arch fat binary (Ampere/Ada/Hopper/Blackwell)
set(CMAKE_CUDA_ARCHITECTURES 86 89 90 120)

option(BUILD_RPC_SERVER "Build cluster RPC server" OFF)

if(BUILD_RPC_SERVER)
    # add_subdirectory(third_party/llama.cpp)
    add_executable(rpc-server-ext src/main.cpp)
    target_compile_features(rpc-server-ext PRIVATE cxx_std_20)
endif()
