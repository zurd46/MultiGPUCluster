//! Precision negotiation across heterogeneous backends.
//!
//! When a CUDA stage talks to a Metal stage over the wire, both sides must
//! emit and accept the same numeric format for activations. The default is
//! BF16 because every Ampere+ NVIDIA GPU and every M3+ Apple GPU supports it
//! natively. FP8 / FP4 narrow the eligible set drastically and are used only
//! when the user explicitly asks for them.

use gpucluster_proto::node as pb;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Precision {
    Fp16,
    Bf16,
    Fp8,
    Fp4,
}

impl Precision {
    pub fn supported_by(&self, cap: &pb::GpuCapabilityProfile) -> bool {
        match self {
            Precision::Fp16 => cap.supports_fp16,
            Precision::Bf16 => cap.supports_bf16,
            Precision::Fp8  => cap.supports_fp8,
            Precision::Fp4  => cap.supports_fp4,
        }
    }
}

/// Cluster-wide greatest-common-denominator: returns the best precision that
/// every supplied GPU can do natively. Used at job-admission time to pick a
/// safe wire format if the user didn't override it.
pub fn cluster_gcd(gpus: &[&pb::GpuInfo]) -> Precision {
    // Start with the most aggressive and walk down until everyone agrees.
    for &p in &[Precision::Fp4, Precision::Fp8, Precision::Bf16, Precision::Fp16] {
        if gpus.iter().all(|g| g.capability.as_ref().map_or(false, |c| p.supported_by(c))) {
            return p;
        }
    }
    Precision::Fp16
}
