use anyhow::Result;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use time::{Duration as TimeDuration, OffsetDateTime};

/// In-memory CA materials. Persist `cert_pem` and `key_pem` securely.
pub struct Ca {
    pub cert: Certificate,
    pub key: KeyPair,
}

impl Ca {
    pub fn cert_pem(&self) -> String { self.cert.pem() }
    pub fn key_pem(&self)  -> String { self.key.serialize_pem() }
}

/// Materials for an issued leaf certificate (e.g. a worker node's mTLS cert).
pub struct IssuedCert {
    pub cert_pem: String,
    pub key_pem: String,
}

pub fn generate_root_ca(common_name: &str) -> Result<Ca> {
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
    Ok(Ca { cert, key })
}

pub fn issue_node_cert(ca: &Ca, node_id: &str, valid_days: u32) -> Result<IssuedCert> {
    let mut params = CertificateParams::new(vec![format!("{node_id}.nodes.cluster.local")])?;
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, node_id);
    dn.push(DnType::OrganizationalUnitName, "worker-node");
    params.distinguished_name = dn;
    params.is_ca = IsCa::NoCa;
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyAgreement,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    params.not_after = OffsetDateTime::now_utc() + TimeDuration::days(valid_days as i64);

    let leaf_key = KeyPair::generate()?;
    let leaf_cert = params.signed_by(&leaf_key, &ca.cert, &ca.key)?;

    Ok(IssuedCert {
        cert_pem: leaf_cert.pem(),
        key_pem: leaf_key.serialize_pem(),
    })
}
