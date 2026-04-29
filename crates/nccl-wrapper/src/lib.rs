use thiserror::Error;

#[derive(Debug, Error)]
pub enum NcclError {
    #[error("nccl not available in this build (enable feature `nccl`)")]
    NotAvailable,
    #[error("nccl error: {0}")]
    Native(String),
}

pub type Result<T> = std::result::Result<T, NcclError>;

pub struct Communicator {
    _private: (),
}

impl Communicator {
    pub fn new(_world_size: u32, _rank: u32) -> Result<Self> {
        #[cfg(feature = "nccl")]
        {
            // TODO: ncclCommInitRank / ncclCommInitAll
            return Err(NcclError::Native("not implemented".into()));
        }
        #[cfg(not(feature = "nccl"))]
        {
            Err(NcclError::NotAvailable)
        }
    }
}
