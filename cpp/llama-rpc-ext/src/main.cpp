// Phase 2: cluster-aware extension of llama.cpp's experimental RPC backend.
//
// Build matrix:
//   GPUCLUSTER_BACKEND_CUDA   → link ggml-cuda  (Linux/Windows worker container)
//   GPUCLUSTER_BACKEND_METAL  → link ggml-metal (macOS / Apple Silicon worker)
//   neither                   → CPU stub, useful for CI smoke tests
//
// TODO (real implementation):
//   1. Pull llama.cpp as a git submodule under third_party/llama.cpp.
//   2. Wrap ggml-rpc with mTLS auth, per-job tenant tagging, and metrics.
//   3. Expose an admin socket for the worker agent to control lifecycle.

#include <cstdio>
#include <cstdlib>
#include <cstring>

namespace {
const char* backend_label() {
#if defined(GPUCLUSTER_BACKEND_METAL)
    return "metal";
#elif defined(GPUCLUSTER_BACKEND_CUDA)
    return "cuda";
#else
    return "cpu-stub";
#endif
}
}

int main(int argc, char** argv) {
    const char* host = "0.0.0.0";
    int port = 50052;
    for (int i = 1; i < argc; ++i) {
        if (std::strcmp(argv[i], "--host") == 0 && i + 1 < argc) host = argv[++i];
        else if (std::strcmp(argv[i], "--port") == 0 && i + 1 < argc) port = std::atoi(argv[++i]);
    }

    std::printf("rpc-server-ext starting (backend=%s, listen=%s:%d) — phase 2 stub\n",
                backend_label(), host, port);
    // Real ggml-rpc bringup goes here once the llama.cpp submodule lands.
    return 0;
}
