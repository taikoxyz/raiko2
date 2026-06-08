use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpcodeLabInput {
    pub case: String,
    pub scenario: String,
    pub opcode: u8,
    pub target_count: u64,
    pub target_raw_gas: u64,
    #[serde(with = "hex_bytes")]
    pub bytecode: Vec<u8>,
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
    use super::OpcodeLabInput;

    #[test]
    fn opcode_lab_input_deserializes_hex_bytecode() {
        let input: OpcodeLabInput = serde_json::from_str(
            r#"{
              "case": "add",
              "scenario": "arithmetic",
              "opcode": 1,
              "target_count": 4,
              "target_raw_gas": 3,
              "bytecode": "0x600160020100"
            }"#,
        )
        .expect("parse lab input");

        assert_eq!(input.bytecode, vec![0x60, 0x01, 0x60, 0x02, 0x01, 0x00]);
    }
}
