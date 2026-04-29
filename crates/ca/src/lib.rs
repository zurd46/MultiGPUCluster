use anyhow::Result;
use rcgen::{
    BasicConstraints, Certificate, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose,
};
use time::{Duration as TimeDuration, OffsetDateTime};

/// Active CA materials for the cluster.
///
/// `original_cert_pem` is the cert that was stored in the database when the CA
/// was first generated — that's the cert workers receive in their CA chain and
/// pin against. `cert` is an in-memory issuer handle used for signing new leaf
/// certs. After a restart the in-memory `cert` is rebuilt with the same DN and
/// same key, so it produces leaves whose `Issuer` field matches the
/// `original_cert_pem`'s `Subject`.
pub struct Ca {
    pub cert: Certificate,
    pub key: KeyPair,
    pub original_cert_pem: Option<String>,
    pub common_name: String,
}

impl Ca {
    pub fn cert_pem(&self) -> String {
        self.original_cert_pem
            .clone()
            .unwrap_or_else(|| self.cert.pem())
    }
    pub fn key_pem(&self) -> String {
        self.key.serialize_pem()
    }
}

pub struct IssuedCert {
    pub cert_pem: String,
    pub key_pem: String,
}

pub fn generate_root_ca(common_name: &str) -> Result<Ca> {
    let key = KeyPair::generate()?;
    let cert = build_root_cert(common_name, &key)?;
    Ok(Ca {
        cert,
        key,
        original_cert_pem: None,
        common_name: common_name.to_string(),
    })
}

/// Re-hydrate a CA from previously persisted PEM material.
/// Fails if the key is unreadable. The original cert PEM is kept verbatim so
/// distributed CA chains stay stable.
pub fn load_ca(common_name: &str, original_cert_pem: &str, key_pem: &str) -> Result<Ca> {
    let key = KeyPair::from_pem(key_pem)?;
    let cert = build_root_cert(common_name, &key)?;
    Ok(Ca {
        cert,
        key,
        original_cert_pem: Some(original_cert_pem.to_string()),
        common_name: common_name.to_string(),
    })
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

fn build_root_cert(common_name: &str, key: &KeyPair) -> Result<Certificate> {
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
    Ok(params.self_signed(key)?)
}
