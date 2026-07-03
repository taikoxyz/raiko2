use alloy_primitives::{B256, hex, keccak256};

const SINK_MAGIC: &str = "raiko2:request-identity:v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestFingerprint(String);

impl RequestFingerprint {
    pub(crate) fn from_identity<T: RequestIdentity + ?Sized>(identity: &T) -> Self {
        let mut sink = FingerprintSink::new(T::DOMAIN);
        identity.write_identity(&mut sink);
        sink.finish()
    }

    pub(crate) fn from_hex(fingerprint: impl Into<String>) -> Self {
        Self(fingerprint.into())
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn public_task_id(&self) -> String {
        let fingerprint = self.0.strip_prefix("0x").unwrap_or(&self.0);
        format!("task_{fingerprint}")
    }
}

pub(crate) trait RequestIdentity {
    const DOMAIN: &'static str;

    fn write_identity(&self, sink: &mut FingerprintSink);

    fn fingerprint(&self) -> RequestFingerprint {
        RequestFingerprint::from_identity(self)
    }
}

pub(crate) struct FingerprintSink {
    bytes: Vec<u8>,
}

impl FingerprintSink {
    pub(crate) fn new(domain: &'static str) -> Self {
        let mut sink = Self { bytes: Vec::new() };
        sink.write_chunk("magic", SINK_MAGIC.as_bytes());
        sink.write_chunk("domain", domain.as_bytes());
        sink
    }

    pub(crate) fn finish(self) -> RequestFingerprint {
        RequestFingerprint(hex::encode_prefixed(keccak256(self.bytes).as_slice()))
    }

    pub(crate) fn bool(&mut self, field: impl AsRef<str>, value: bool) {
        self.field(field, "bool", &[u8::from(value)]);
    }

    pub(crate) fn b256(&mut self, field: impl AsRef<str>, value: &B256) {
        self.field(field, "b256", value.as_slice());
    }

    pub(crate) fn str(&mut self, field: impl AsRef<str>, value: &str) {
        self.field(field, "str", value.as_bytes());
    }

    pub(crate) fn opt_str(&mut self, field: impl AsRef<str>, value: Option<&str>) {
        let field = field.as_ref();
        self.bool(format!("{field}.present"), value.is_some());
        if let Some(value) = value {
            self.str(format!("{field}.value"), value);
        }
    }

    pub(crate) fn u64(&mut self, field: impl AsRef<str>, value: u64) {
        self.field(field, "u64", &value.to_be_bytes());
    }

    fn field(&mut self, field: impl AsRef<str>, kind: &'static str, value: &[u8]) {
        self.write_chunk("field", field.as_ref().as_bytes());
        self.write_chunk("kind", kind.as_bytes());
        self.write_chunk("value", value);
    }

    fn write_chunk(&mut self, tag: &'static str, value: &[u8]) {
        self.bytes
            .extend_from_slice(&(tag.len() as u64).to_be_bytes());
        self.bytes.extend_from_slice(tag.as_bytes());
        self.bytes
            .extend_from_slice(&(value.len() as u64).to_be_bytes());
        self.bytes.extend_from_slice(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ExampleIdentity {
        value: u64,
    }

    impl RequestIdentity for ExampleIdentity {
        const DOMAIN: &'static str = "example";

        fn write_identity(&self, sink: &mut FingerprintSink) {
            sink.u64("value", self.value);
        }
    }

    #[test]
    fn fingerprint_is_stable_and_task_id_uses_v3_shape() {
        let first = ExampleIdentity { value: 7 }.fingerprint();
        let second = ExampleIdentity { value: 7 }.fingerprint();
        let changed = ExampleIdentity { value: 8 }.fingerprint();

        assert_eq!(first, second);
        assert_ne!(first, changed);
        assert!(first.as_str().starts_with("0x"));
        assert_eq!(first.public_task_id().len(), "task_".len() + 64);
        assert!(first.public_task_id().starts_with("task_"));
    }
}
