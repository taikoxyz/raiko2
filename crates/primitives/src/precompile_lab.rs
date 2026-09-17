use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrecompileLabLane {
    #[default]
    Target,
    Control,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PrecompileLabInput {
    pub case: String,
    pub scenario: String,
    #[serde(default)]
    pub lane: PrecompileLabLane,
    pub address: u8,
    pub target_count: u64,
    pub input_size: u64,
    pub target_raw_gas: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_output_size: Option<u64>,
    #[serde(with = "hex_bytes")]
    pub input: Vec<u8>,
}

impl PrecompileLabInput {
    /// Validates the typed target/control contract used by paired controlled measurements.
    ///
    /// # Errors
    ///
    /// Returns an error when the declared input size is wrong or a control lane omits the output
    /// size required to preserve the target lane's folding shape.
    pub fn validate_controlled_contract(&self) -> Result<(), &'static str> {
        if u64::try_from(self.input.len()).unwrap_or(u64::MAX) != self.input_size {
            return Err("precompile input length does not match input_size");
        }
        if self.lane == PrecompileLabLane::Control && self.expected_output_size.is_none() {
            return Err("control lane requires expected_output_size");
        }
        Ok(())
    }
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
    use super::{PrecompileLabInput, PrecompileLabLane};

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
        assert_eq!(input.lane, PrecompileLabLane::Target);
    }

    #[test]
    fn precompile_control_lane_is_typed_and_requires_a_frozen_output_size() {
        let input: PrecompileLabInput = serde_json::from_str(
            r#"{
              "case": "identity-control",
              "scenario": "free text must not select the lane",
              "lane": "control",
              "address": 4,
              "target_count": 2,
              "input_size": 2,
              "target_raw_gas": 18,
              "expected_output_size": 2,
              "input": "0x0102"
            }"#,
        )
        .expect("parse typed control input");

        assert_eq!(input.lane, PrecompileLabLane::Control);
        input.validate_controlled_contract().unwrap();

        let missing_shape = PrecompileLabInput {
            expected_output_size: None,
            ..input
        };
        assert_eq!(
            missing_shape.validate_controlled_contract(),
            Err("control lane requires expected_output_size")
        );
    }
}
