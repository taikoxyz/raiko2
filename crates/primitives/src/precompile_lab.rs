use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrecompileLabInput {
    pub case: String,
    pub scenario: String,
    pub address: u8,
    pub target_count: u64,
    pub input_size: u64,
    pub target_raw_gas: u64,
    #[serde(with = "hex_bytes")]
    pub input: Vec<u8>,
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", alloy_primitives::hex::encode(bytes)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        let value = value.strip_prefix("0x").unwrap_or(&value);
        alloy_primitives::hex::decode(value).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::PrecompileLabInput;

    #[test]
    fn precompile_lab_input_deserializes_hex_input() {
        let input: PrecompileLabInput = serde_json::from_str(
            r#"{
              "case": "identity",
              "scenario": "precompile",
              "address": 4,
              "target_count": 2,
              "input_size": 32,
              "target_raw_gas": 18,
              "input": "0x0102"
            }"#,
        )
        .expect("parse lab input");

        assert_eq!(input.input, vec![0x01, 0x02]);
    }
}
