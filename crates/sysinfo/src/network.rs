use gpucluster_proto::node as pb;

pub fn detect() -> pb::NetworkInfo {
    pb::NetworkInfo {
        public_ip_v4: String::new(),
        public_ip_v6: String::new(),
        asn: String::new(),
        isp: String::new(),
        country_code: String::new(),
        city: String::new(),
        public_ip_is_dynamic: false,
        public_ip_changed_at: 0,
        local_ips: collect_local_ips(),
        primary_iface: String::new(),
        link_speed_mbps: 0,
        wg_ip: String::new(),
        wg_pubkey_sha: String::new(),
        wg_listen_port: 0,
        rtt_to_gateway_ms: 0,
    }
}

fn collect_local_ips() -> Vec<String> {
    use std::net::UdpSocket;
    let mut out = Vec::new();
    if let Ok(s) = UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = s.local_addr() {
                out.push(addr.ip().to_string());
            }
        }
    }
    out
}
