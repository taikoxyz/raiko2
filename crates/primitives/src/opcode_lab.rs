use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpcodeLabInput {
    pub case: String,
    pub scenario: String,
    pub opcode: u8,
    pub target_count: u64,
    pub target_raw_gas: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tx_gas_limit: Option<u64>,
    #[serde(with = "hex_bytes")]
    pub bytecode: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator_max_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fixed_bytecode_len: Option<u64>,
}

impl OpcodeLabInput {
    pub const GAS_LIMIT_OVERHEAD: u64 = 1_000_000;
    pub const MIN_EXECUTION_GAS_LIMIT: u64 = 100_000;
    pub const FIXED_MICROPROGRAM_MAGIC: [u8; 4] = [0xef, 0x4d, 0x50, 0x01];

    #[must_use]
    pub const fn execution_gas_limit(&self) -> u64 {
        match self.tx_gas_limit {
            Some(tx_gas_limit) => tx_gas_limit,
            None => self
                .target_raw_gas
                .saturating_mul(self.target_count)
                .saturating_add(Self::GAS_LIMIT_OVERHEAD),
        }
    }

    /// Validates the count and bytecode-size commitments carried by a controlled fixture.
    ///
    /// # Errors
    ///
    /// Returns an error when the selected count exceeds the declared generator bound or the
    /// serialized bytecode length differs from the frozen layout length.
    pub fn validate_controlled_contract(&self) -> Result<(), &'static str> {
        if let Some(max_count) = self.generator_max_count
            && self.target_count > max_count
        {
            return Err("target_count exceeds declared generator_max_count");
        }
        if self.generator_max_count.is_some() && self.tx_gas_limit.is_none() {
            return Err("controlled input is missing tx_gas_limit");
        }
        if self
            .tx_gas_limit
            .is_some_and(|gas_limit| gas_limit < Self::MIN_EXECUTION_GAS_LIMIT)
        {
            return Err("tx_gas_limit is below the revm opcode execution minimum");
        }
        if let Some(fixed_len) = self.fixed_bytecode_len
            && u64::try_from(self.bytecode.len()).unwrap_or(u64::MAX) != fixed_len
        {
            return Err("bytecode length differs from fixed_bytecode_len");
        }
        self.execution_programs()?;
        Ok(())
    }

    /// Returns the one legacy program or every framed fixed-footprint microprogram.
    ///
    /// # Errors
    ///
    /// Returns an error when the framed microprogram header, count, or length table is malformed,
    /// or when its program count differs from the declared generator footprint.
    pub fn execution_programs(&self) -> Result<Vec<&[u8]>, &'static str> {
        if !self.bytecode.starts_with(&Self::FIXED_MICROPROGRAM_MAGIC) {
            return Ok(vec![self.bytecode.as_slice()]);
        }
        if self.bytecode.len() < 8 {
            return Err("truncated fixed-microprogram header");
        }
        let count = u32::from_be_bytes(
            self.bytecode[4..8]
                .try_into()
                .map_err(|_| "truncated fixed-microprogram header")?,
        );
        if count == 0 {
            return Err("fixed microprogram list is empty");
        }
        if let Some(max_count) = self.generator_max_count
            && u64::from(count) != max_count
        {
            return Err("fixed microprogram count differs from generator_max_count");
        }
        let mut cursor = 8usize;
        let mut programs = Vec::with_capacity(count as usize);
        for _ in 0..count {
            if cursor
                .checked_add(4)
                .is_none_or(|end| end > self.bytecode.len())
            {
                return Err("truncated fixed-microprogram length");
            }
            let length = u32::from_be_bytes(
                self.bytecode[cursor..cursor + 4]
                    .try_into()
                    .map_err(|_| "truncated fixed-microprogram length")?,
            ) as usize;
            cursor += 4;
            let end = cursor
                .checked_add(length)
                .ok_or("fixed microprogram length overflow")?;
            if end > self.bytecode.len() {
                return Err("truncated fixed microprogram");
            }
            programs.push(&self.bytecode[cursor..end]);
            cursor = end;
        }
        if cursor != self.bytecode.len() {
            return Err("trailing fixed-microprogram bytes");
        }
        Ok(programs)
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
        assert_eq!(input.generator_max_count, None);
        assert_eq!(input.fixed_bytecode_len, None);
    }

    #[test]
    fn controlled_contract_binds_generator_bound_and_fixed_bytecode_length() {
        let input: OpcodeLabInput = serde_json::from_str(
            r#"{
              "case": "add",
              "scenario": "arithmetic",
              "opcode": 1,
              "target_count": 4,
              "target_raw_gas": 3,
              "tx_gas_limit": 1000024,
              "bytecode": "0x600160020100",
              "generator_max_count": 8,
              "fixed_bytecode_len": 6
            }"#,
        )
        .expect("parse controlled lab input");

        input
            .validate_controlled_contract()
            .expect("valid controlled contract");
        assert_eq!(input.tx_gas_limit, Some(1_000_024));
        assert_eq!(input.execution_gas_limit(), 1_000_024);

        let lower_count = OpcodeLabInput {
            target_count: 1,
            ..input.clone()
        };
        assert_eq!(lower_count.execution_gas_limit(), 1_000_024);

        let beyond_bound = OpcodeLabInput {
            target_count: 9,
            ..input.clone()
        };
        assert_eq!(
            beyond_bound.validate_controlled_contract(),
            Err("target_count exceeds declared generator_max_count")
        );

        let wrong_size = OpcodeLabInput {
            bytecode: vec![0x00],
            ..input
        };
        assert_eq!(
            wrong_size.validate_controlled_contract(),
            Err("bytecode length differs from fixed_bytecode_len")
        );
    }

    #[test]
    fn controlled_contract_requires_explicit_transaction_gas_limit() {
        let input = OpcodeLabInput {
            target_count: 4,
            target_raw_gas: 3,
            generator_max_count: Some(8),
            ..OpcodeLabInput::default()
        };

        assert_eq!(
            input.validate_controlled_contract(),
            Err("controlled input is missing tx_gas_limit")
        );
    }

    #[test]
    fn controlled_contract_rejects_transaction_gas_limit_below_execution_minimum() {
        let input = OpcodeLabInput {
            target_count: 4,
            target_raw_gas: 3,
            tx_gas_limit: Some(99_999),
            generator_max_count: Some(8),
            ..OpcodeLabInput::default()
        };

        assert_eq!(
            input.validate_controlled_contract(),
            Err("tx_gas_limit is below the revm opcode execution minimum")
        );
    }

    #[test]
    fn controlled_contract_decodes_fixed_microprogram_framing() {
        let input = OpcodeLabInput {
            bytecode: vec![
                0xef, 0x4d, 0x50, 0x01, 0, 0, 0, 2, 0, 0, 0, 2, 0x01, 0x00, 0, 0, 0, 2, 0x00, 0x01,
            ],
            generator_max_count: Some(2),
            fixed_bytecode_len: Some(20),
            ..Default::default()
        };

        assert_eq!(
            input.execution_programs().unwrap(),
            vec![&[0x01, 0x00][..], &[0x00, 0x01][..]]
        );

        let mut truncated = input;
        truncated.bytecode.pop();
        assert_eq!(
            truncated.execution_programs(),
            Err("truncated fixed microprogram")
        );
    }
}
