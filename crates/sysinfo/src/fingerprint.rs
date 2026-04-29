use anyhow::Result;

pub fn compute() -> Result<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();

    if let Ok(Some(mac)) = mac_address::get_mac_address() {
        mac.bytes().hash(&mut hasher);
    }

    std::env::consts::ARCH.hash(&mut hasher);
    std::env::consts::OS.hash(&mut hasher);

    let v = hasher.finish();
    Ok(format!("hwfp-{v:016x}"))
}
