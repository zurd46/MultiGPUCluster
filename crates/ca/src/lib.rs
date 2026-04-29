use anyhow::{anyhow, Result};
use rcgen::{
    CertificateParams, DistinguishedName, DnType, IsCa, KeyPair, KeyUsagePurpose,
    ExtendedKeyUsagePurpose, BasicConstraints,
};

pub struct CaBundle {
    pub cert_pem: String,
    pub key_pem: String,
}

pub fn generate_root_ca(common_name: &str) -> Result<CaBundle> {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, common_name);
    dn.push(DnType::OrganizationName, "MultiGPUCluster");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    Ok(CaBundle {
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

pub fn issue_node_cert(
    ca: &CaBundle,
    node_id: &str,
    valid_days: u32,
) -> Result<CaBundle> {
    let ca_key = KeyPair::from_pem(&ca.key_pem)?;
    let ca_params = CertificateParams::from_ca_cert_pem(&ca.cert_pem)?;
    let ca_cert = ca_params.self_signed(&ca_key)?;

    let mut params = CertificateParams::new(vec![format!("{node_id}.nodes.cluster.local")])?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, node_id);
    dn.push(DnType::OrganizationalUnitName, "worker-node");
    params.distinguished_name = dn;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyAgreement];
    params.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::ClientAuth,
    ];
    params.not_after = time::OffsetDateTime::now_utc()
        .checked_add(time::Duration::days(valid_days as i64))
        .ok_or_else(|| anyhow!("invalid expiry"))?;

    let node_key = KeyPair::generate()?;
    let cert = params.signed_by(&node_key, &ca_cert, &ca_key)?;

    Ok(CaBundle {
        cert_pem: cert.pem(),
        key_pem: node_key.serialize_pem(),
    })
}
