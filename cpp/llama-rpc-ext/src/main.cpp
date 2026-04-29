// Phase 2: cluster-aware extension of llama.cpp's experimental RPC backend.
//
// This file is a placeholder that compiles standalone so the project builds
// in its early stages. The real implementation will:
//   1. Pull llama.cpp as a git submodule under third_party/llama.cpp.
//   2. Wrap ggml-rpc with mTLS auth, per-job tenant tagging, and metrics.
//   3. Expose an admin socket for the worker agent to control lifecycle.

#include <cstdio>

int main(int argc, char** argv) {
    (void)argc; (void)argv;
    std::printf("rpc-server-ext stub — phase 2 implementation pending\n");
    return 0;
}
