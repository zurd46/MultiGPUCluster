cmake_minimum_required(VERSION 3.20)
project(gpucluster_cuda_kernels LANGUAGES CXX CUDA)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CUDA_STANDARD 17)
set(CMAKE_CUDA_ARCHITECTURES 86 89 90 120)

# Phase 4: custom kernels for fine-tuning ops not provided by upstream stacks.
add_library(gpucluster_kernels STATIC src/placeholder.cu)
target_compile_features(gpucluster_kernels PUBLIC cxx_std_20)
