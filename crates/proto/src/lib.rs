pub mod node {
    tonic::include_proto!("gpucluster.node.v1");
}

pub mod coordinator {
    tonic::include_proto!("gpucluster.coordinator.v1");
}

pub mod management {
    tonic::include_proto!("gpucluster.management.v1");
}
