use std::path::PathBuf;

pub mod rtt;
pub mod serial;

#[derive(Clone, Debug)]
pub enum TransportSpec {
    Loopback,
    Rtt,
    Serial(PathBuf),
}

pub fn parse_transport(s: &str) -> Result<TransportSpec, String> {
    match s {
        "loopback" => Ok(TransportSpec::Loopback),
        "rtt" => Ok(TransportSpec::Rtt),
        other => match other.strip_prefix("serial:") {
            Some(p) if !p.is_empty() => Ok(TransportSpec::Serial(PathBuf::from(p))),
            Some(_) => Err(format!(
                "transport 'serial:' requires a non-empty path (got '{other}')"
            )),
            None => Err(format!(
                "unknown transport '{other}': expected 'loopback' | 'rtt' | 'serial:<path>'"
            )),
        },
    }
}
