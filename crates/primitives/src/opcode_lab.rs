use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpcodeLabStorageLane {
    Target,
    Control,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpcodeLabStorageAccess {
    Cold,
    Warm,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpcodeLabStorageOperation {
    Load {
        expected_value: [u8; 32],
    },
    Store {
        current_value: [u8; 32],
        new_value: [u8; 32],
    },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ReadableStorageOperationRef<'a> {
    Load {
        #[serde(with = "hex_word")]
        expected_value: &'a [u8; 32],
    },
    Store {
        #[serde(with = "hex_word")]
        current_value: &'a [u8; 32],
        #[serde(with = "hex_word")]
        new_value: &'a [u8; 32],
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ReadableStorageOperation {
    Load {
        #[serde(with = "hex_word")]
        expected_value: [u8; 32],
    },
    Store {
        #[serde(with = "hex_word")]
        current_value: [u8; 32],
        #[serde(with = "hex_word")]
        new_value: [u8; 32],
    },
}

#[derive(Serialize)]
enum BinaryStorageOperationRef<'a> {
    Load(&'a [u8; 32]),
    Store(&'a [u8; 32], &'a [u8; 32]),
}

#[derive(Deserialize)]
enum BinaryStorageOperation {
    Load([u8; 32]),
    Store([u8; 32], [u8; 32]),
}

impl Serialize for OpcodeLabStorageOperation {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            match self {
                Self::Load { expected_value } => {
                    ReadableStorageOperationRef::Load { expected_value }.serialize(serializer)
                }
                Self::Store {
                    current_value,
                    new_value,
                } => ReadableStorageOperationRef::Store {
                    current_value,
                    new_value,
                }
                .serialize(serializer),
            }
        } else {
            match self {
                Self::Load { expected_value } => {
                    BinaryStorageOperationRef::Load(expected_value).serialize(serializer)
                }
                Self::Store {
                    current_value,
                    new_value,
                } => {
                    BinaryStorageOperationRef::Store(current_value, new_value).serialize(serializer)
                }
            }
        }
    }
}

impl<'de> Deserialize<'de> for OpcodeLabStorageOperation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            Ok(match ReadableStorageOperation::deserialize(deserializer)? {
                ReadableStorageOperation::Load { expected_value } => Self::Load { expected_value },
                ReadableStorageOperation::Store {
                    current_value,
                    new_value,
                } => Self::Store {
                    current_value,
                    new_value,
                },
            })
        } else {
            Ok(match BinaryStorageOperation::deserialize(deserializer)? {
                BinaryStorageOperation::Load(expected_value) => Self::Load { expected_value },
                BinaryStorageOperation::Store(current_value, new_value) => Self::Store {
                    current_value,
                    new_value,
                },
            })
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct OpcodeLabStorageInput {
    pub measurement_opcode: u8,
    pub lane: OpcodeLabStorageLane,
    #[serde(with = "hex_word")]
    pub slot: [u8; 32],
    #[serde(with = "hex_word")]
    pub original_value: [u8; 32],
    pub access: OpcodeLabStorageAccess,
    pub operation: OpcodeLabStorageOperation,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpcodeLabInput {
    pub case: String,
    pub scenario: String,
    pub opcode: u8,
    pub target_count: u64,
    pub target_raw_gas: u64,
    pub tx_gas_limit: Option<u64>,
    pub bytecode: Vec<u8>,
    pub generator_max_count: Option<u64>,
    pub fixed_bytecode_len: Option<u64>,
    pub storage: Option<OpcodeLabStorageInput>,
    pub tx_value: [u8; 32],
    pub calldata: Vec<u8>,
    pub block_timestamp: Option<u64>,
}

fn word_is_zero(value: &[u8; 32]) -> bool {
    *value == [0; 32]
}

#[derive(Serialize)]
struct ReadableOpcodeLabInputRef<'a> {
    case: &'a str,
    scenario: &'a str,
    opcode: u8,
    target_count: u64,
    target_raw_gas: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tx_gas_limit: Option<u64>,
    #[serde(with = "hex_bytes")]
    bytecode: &'a [u8],
    #[serde(skip_serializing_if = "Option::is_none")]
    generator_max_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fixed_bytecode_len: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    storage: Option<&'a OpcodeLabStorageInput>,
    #[serde(with = "hex_word", skip_serializing_if = "word_is_zero")]
    tx_value: &'a [u8; 32],
    #[serde(with = "hex_bytes", skip_serializing_if = "<[u8]>::is_empty")]
    calldata: &'a [u8],
    #[serde(skip_serializing_if = "Option::is_none")]
    block_timestamp: Option<u64>,
}

#[derive(Deserialize)]
struct ReadableOpcodeLabInput {
    case: String,
    scenario: String,
    opcode: u8,
    target_count: u64,
    target_raw_gas: u64,
    #[serde(default)]
    tx_gas_limit: Option<u64>,
    #[serde(with = "hex_bytes")]
    bytecode: Vec<u8>,
    #[serde(default)]
    generator_max_count: Option<u64>,
    #[serde(default)]
    fixed_bytecode_len: Option<u64>,
    #[serde(default)]
    storage: Option<OpcodeLabStorageInput>,
    #[serde(default, with = "hex_word")]
    tx_value: [u8; 32],
    #[serde(default, with = "hex_bytes")]
    calldata: Vec<u8>,
    #[serde(default)]
    block_timestamp: Option<u64>,
}

#[derive(Serialize)]
struct BinaryOpcodeLabInputRef<'a> {
    case: &'a str,
    scenario: &'a str,
    opcode: u8,
    target_count: u64,
    target_raw_gas: u64,
    tx_gas_limit: Option<u64>,
    bytecode: &'a [u8],
    generator_max_count: Option<u64>,
    fixed_bytecode_len: Option<u64>,
    storage: Option<&'a OpcodeLabStorageInput>,
    tx_value: &'a [u8; 32],
    calldata: &'a [u8],
    block_timestamp: Option<u64>,
}

#[derive(Deserialize)]
struct BinaryOpcodeLabInput {
    case: String,
    scenario: String,
    opcode: u8,
    target_count: u64,
    target_raw_gas: u64,
    tx_gas_limit: Option<u64>,
    bytecode: Vec<u8>,
    generator_max_count: Option<u64>,
    fixed_bytecode_len: Option<u64>,
    storage: Option<OpcodeLabStorageInput>,
    tx_value: [u8; 32],
    calldata: Vec<u8>,
    block_timestamp: Option<u64>,
}

impl Serialize for OpcodeLabInput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if serializer.is_human_readable() {
            ReadableOpcodeLabInputRef {
                case: &self.case,
                scenario: &self.scenario,
                opcode: self.opcode,
                target_count: self.target_count,
                target_raw_gas: self.target_raw_gas,
                tx_gas_limit: self.tx_gas_limit,
                bytecode: &self.bytecode,
                generator_max_count: self.generator_max_count,
                fixed_bytecode_len: self.fixed_bytecode_len,
                storage: self.storage.as_ref(),
                tx_value: &self.tx_value,
                calldata: &self.calldata,
                block_timestamp: self.block_timestamp,
            }
            .serialize(serializer)
        } else {
            BinaryOpcodeLabInputRef {
                case: &self.case,
                scenario: &self.scenario,
                opcode: self.opcode,
                target_count: self.target_count,
                target_raw_gas: self.target_raw_gas,
                tx_gas_limit: self.tx_gas_limit,
                bytecode: &self.bytecode,
                generator_max_count: self.generator_max_count,
                fixed_bytecode_len: self.fixed_bytecode_len,
                storage: self.storage.as_ref(),
                tx_value: &self.tx_value,
                calldata: &self.calldata,
                block_timestamp: self.block_timestamp,
            }
            .serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for OpcodeLabInput {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if deserializer.is_human_readable() {
            let input = ReadableOpcodeLabInput::deserialize(deserializer)?;
            Ok(Self {
                case: input.case,
                scenario: input.scenario,
                opcode: input.opcode,
                target_count: input.target_count,
                target_raw_gas: input.target_raw_gas,
                tx_gas_limit: input.tx_gas_limit,
                bytecode: input.bytecode,
                generator_max_count: input.generator_max_count,
                fixed_bytecode_len: input.fixed_bytecode_len,
                storage: input.storage,
                tx_value: input.tx_value,
                calldata: input.calldata,
                block_timestamp: input.block_timestamp,
            })
        } else {
            let input = BinaryOpcodeLabInput::deserialize(deserializer)?;
            Ok(Self {
                case: input.case,
                scenario: input.scenario,
                opcode: input.opcode,
                target_count: input.target_count,
                target_raw_gas: input.target_raw_gas,
                tx_gas_limit: input.tx_gas_limit,
                bytecode: input.bytecode,
                generator_max_count: input.generator_max_count,
                fixed_bytecode_len: input.fixed_bytecode_len,
                storage: input.storage,
                tx_value: input.tx_value,
                calldata: input.calldata,
                block_timestamp: input.block_timestamp,
            })
        }
    }
}

impl OpcodeLabInput {
    pub const DEFAULT_BLOCK_TIMESTAMP: u64 = 1;
    pub const GAS_LIMIT_OVERHEAD: u64 = 1_000_000;
    pub const MIN_EXECUTION_GAS_LIMIT: u64 = 100_000;
    pub const FIXED_MICROPROGRAM_MAGIC: [u8; 4] = [0xef, 0x4d, 0x50, 0x01];

    #[must_use]
    pub const fn effective_block_timestamp(&self) -> u64 {
        match self.block_timestamp {
            Some(timestamp) => timestamp,
            None => Self::DEFAULT_BLOCK_TIMESTAMP,
        }
    }

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
        if let Some(max_count) = self.generator_max_count {
            let max_declared_opcode_count = match self.storage.as_ref() {
                Some(OpcodeLabStorageInput {
                    measurement_opcode: 0x55,
                    lane: OpcodeLabStorageLane::Control,
                    ..
                }) => max_count.saturating_mul(2),
                _ => max_count,
            };
            if self.target_count > max_declared_opcode_count {
                return Err("target_count exceeds declared generator_max_count");
            }
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
        self.validate_storage_contract()?;
        self.execution_programs()?;
        Ok(())
    }

    /// Validates the typed storage scenario against the concrete lane and bytecode.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage operation, lane opcode, canonical operands, access class,
    /// or dirty prefix differs from the declared contract.
    pub fn validate_storage_contract(&self) -> Result<(), &'static str> {
        let Some(storage) = &self.storage else {
            return Ok(());
        };
        match storage.measurement_opcode {
            0x54 | 0x55 => {}
            _ => return Err("storage measurement_opcode must be SLOAD or SSTORE"),
        }
        match storage.lane {
            OpcodeLabStorageLane::Target if self.opcode != storage.measurement_opcode => {
                return Err("target opcode differs from storage measurement_opcode");
            }
            OpcodeLabStorageLane::Control if matches!(self.opcode, 0x54 | 0x55) => {
                return Err("control opcode must name a non-storage reference opcode");
            }
            _ => {}
        }

        let programs = self.execution_programs()?;
        match &storage.operation {
            OpcodeLabStorageOperation::Load { expected_value } => {
                if storage.measurement_opcode != 0x54 {
                    return Err("load operation requires SLOAD measurement_opcode");
                }
                if expected_value != &storage.original_value {
                    return Err("SLOAD expected_value differs from original_value");
                }
                self.validate_load_programs(storage, &programs)
            }
            OpcodeLabStorageOperation::Store {
                current_value,
                new_value,
            } => {
                if storage.measurement_opcode != 0x55 {
                    return Err("store operation requires SSTORE measurement_opcode");
                }
                let dirty = current_value != &storage.original_value;
                if dirty && storage.access != OpcodeLabStorageAccess::Warm {
                    return Err("dirty SSTORE measured operation must be warm");
                }
                self.validate_store_programs(storage, &programs, current_value, new_value, dirty)
            }
        }
    }

    fn validate_load_programs(
        &self,
        storage: &OpcodeLabStorageInput,
        programs: &[&[u8]],
    ) -> Result<(), &'static str> {
        let mut measured_count = 0u64;
        for program in programs {
            let sites = storage_sites(program)?;
            if sites.len() > 1 {
                return Err("microprogram contains multiple measured storage opcodes");
            }
            for site in sites {
                if site.opcode != 0x54 || site.slot != storage.slot {
                    return Err("storage bytecode differs from declared SLOAD operation");
                }
                measured_count = measured_count
                    .checked_add(1)
                    .ok_or("storage opcode count overflow")?;
            }
        }
        if storage.lane == OpcodeLabStorageLane::Control && measured_count != 0 {
            return Err("control lane contains an undeclared storage opcode");
        }
        if storage.lane == OpcodeLabStorageLane::Control {
            if self.opcode != 0x19 {
                return Err("SLOAD control opcode must declare NOT");
            }
            if self.target_raw_gas != 3 {
                return Err("SLOAD control target_raw_gas must equal NOT raw gas");
            }
            let reference_count = decoded_opcode_count(programs, self.opcode)?;
            if reference_count != self.target_count {
                return Err("control opcode count differs from target_count");
            }
        }
        match storage.lane {
            OpcodeLabStorageLane::Target if measured_count != self.target_count => {
                Err("target SLOAD count differs from target_count")
            }
            _ => Ok(()),
        }
    }

    fn validate_store_programs(
        &self,
        storage: &OpcodeLabStorageInput,
        programs: &[&[u8]],
        current_value: &[u8; 32],
        new_value: &[u8; 32],
        dirty: bool,
    ) -> Result<(), &'static str> {
        let mut measured_count = 0u64;
        for program in programs {
            let sites = storage_sites(program)?;
            let measured_start = usize::from(dirty);
            if sites.len().saturating_sub(measured_start) > 1 {
                return Err("microprogram contains multiple measured storage opcodes");
            }
            if dirty {
                let prefix = sites
                    .first()
                    .ok_or("dirty SSTORE bytecode is missing its declared prefix")?;
                if prefix.opcode != 0x55 || prefix.value.as_ref() != Some(current_value) {
                    return Err("SSTORE dirty prefix differs from declared current value");
                }
                if prefix.slot != storage.slot {
                    return Err("SSTORE dirty prefix differs from declared slot");
                }
            }
            for site in &sites[measured_start..] {
                if site.opcode != 0x55
                    || site.slot != storage.slot
                    || site.value.as_ref() != Some(new_value)
                {
                    return Err("storage bytecode differs from declared SSTORE operation");
                }
                measured_count = measured_count
                    .checked_add(1)
                    .ok_or("storage opcode count overflow")?;
            }
        }
        if storage.lane == OpcodeLabStorageLane::Control && measured_count != 0 {
            return Err("control lane contains an undeclared storage opcode");
        }
        if storage.lane == OpcodeLabStorageLane::Control {
            if self.opcode != 0x50 {
                return Err("SSTORE control opcode must declare POP");
            }
            if self.target_raw_gas != 2 {
                return Err("SSTORE control target_raw_gas must equal POP raw gas");
            }
            let reference_count = decoded_opcode_count(programs, self.opcode)?;
            if reference_count != self.target_count {
                return Err("control opcode count differs from target_count");
            }
        }
        match storage.lane {
            OpcodeLabStorageLane::Target if measured_count != self.target_count => {
                Err("target SSTORE count differs from target_count")
            }
            _ => Ok(()),
        }
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

#[derive(Clone, Copy)]
struct StorageSite {
    opcode: u8,
    value: Option<[u8; 32]>,
    slot: [u8; 32],
}

fn storage_sites(program: &[u8]) -> Result<Vec<StorageSite>, &'static str> {
    let instructions = decoded_instructions(program)?;
    let mut sites = Vec::new();
    for (index, (opcode, _)) in instructions.iter().enumerate() {
        match *opcode {
            0x54 => {
                let slot = instructions
                    .get(index.wrapping_sub(1))
                    .and_then(|(_, immediate)| *immediate)
                    .ok_or("storage opcode is missing canonical PUSH32 operands")?;
                sites.push(StorageSite {
                    opcode: *opcode,
                    value: None,
                    slot,
                });
            }
            0x55 => {
                let value = instructions
                    .get(index.wrapping_sub(2))
                    .and_then(|(_, immediate)| *immediate)
                    .ok_or("storage opcode is missing canonical PUSH32 operands")?;
                let slot = instructions
                    .get(index.wrapping_sub(1))
                    .and_then(|(_, immediate)| *immediate)
                    .ok_or("storage opcode is missing canonical PUSH32 operands")?;
                sites.push(StorageSite {
                    opcode: *opcode,
                    value: Some(value),
                    slot,
                });
            }
            _ => {}
        }
    }
    Ok(sites)
}

fn decoded_opcode_count(programs: &[&[u8]], opcode: u8) -> Result<u64, &'static str> {
    let mut count = 0u64;
    for program in programs {
        for (actual, _) in decoded_instructions(program)? {
            if actual == opcode {
                count = count
                    .checked_add(1)
                    .ok_or("control opcode count overflow")?;
            }
        }
    }
    Ok(count)
}

type DecodedInstruction = (u8, Option<[u8; 32]>);

fn decoded_instructions(program: &[u8]) -> Result<Vec<DecodedInstruction>, &'static str> {
    let mut instructions: Vec<DecodedInstruction> = Vec::new();
    let mut cursor = 0usize;
    while cursor < program.len() {
        let opcode = program[cursor];
        cursor += 1;
        if (0x60..=0x7f).contains(&opcode) {
            let immediate_len = usize::from(opcode - 0x5f);
            let end = cursor
                .checked_add(immediate_len)
                .ok_or("PUSH immediate length overflow")?;
            if end > program.len() {
                return Err("truncated PUSH immediate in storage bytecode");
            }
            let immediate = if opcode == 0x7f {
                Some(
                    program[cursor..end]
                        .try_into()
                        .map_err(|_| "invalid PUSH32 immediate")?,
                )
            } else {
                None
            };
            instructions.push((opcode, immediate));
            cursor = end;
        } else {
            instructions.push((opcode, None));
        }
    }

    Ok(instructions)
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

mod hex_word {
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(word: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("0x{}", alloy_primitives::hex::encode(word)))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.len() != 66
            || !value.starts_with("0x")
            || !value[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(D::Error::custom(
                "expected canonical 0x-prefixed 32-byte lowercase hex value",
            ));
        }
        let decoded = alloy_primitives::hex::decode(&value[2..]).map_err(D::Error::custom)?;
        decoded
            .try_into()
            .map_err(|_| D::Error::custom("expected exactly 32 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::OpcodeLabInput;

    #[test]
    fn opcode_lab_context_defaults_preserve_readable_fixture_shape() {
        let input: OpcodeLabInput = serde_json::from_str(
            r#"{
              "case": "address",
              "scenario": "canonical",
              "opcode": 48,
              "target_count": 1,
              "target_raw_gas": 2,
              "bytecode": "0x3000"
            }"#,
        )
        .expect("parse legacy-shaped lab input");

        assert_eq!(input.tx_value, [0; 32]);
        assert!(input.calldata.is_empty());
        assert_eq!(input.block_timestamp, None);
        assert_eq!(input.effective_block_timestamp(), 1);
        let serialized = serde_json::to_value(&input).expect("serialize default context");
        assert!(serialized.get("tx_value").is_none());
        assert!(serialized.get("calldata").is_none());
        assert!(serialized.get("block_timestamp").is_none());
    }

    #[test]
    fn opcode_lab_context_is_canonical_json_and_bincode_identity_input() {
        let input: OpcodeLabInput = serde_json::from_str(
            r#"{
              "case": "calldataload",
              "scenario": "partial",
              "opcode": 53,
              "target_count": 1,
              "target_raw_gas": 3,
              "bytecode": "0x5f3500",
              "tx_value": "0x0000000000000000000000000000000000000000000000000000000000000007",
              "calldata": "0x010203",
              "block_timestamp": 17
            }"#,
        )
        .expect("parse explicit context");

        assert_eq!(input.tx_value[31], 7);
        assert_eq!(input.calldata, vec![1, 2, 3]);
        assert_eq!(input.block_timestamp, Some(17));
        let serialized = serde_json::to_value(&input).expect("serialize explicit context");
        assert_eq!(
            serialized["tx_value"],
            "0x0000000000000000000000000000000000000000000000000000000000000007"
        );
        assert_eq!(serialized["calldata"], "0x010203");
        assert_eq!(serialized["block_timestamp"], 17);

        let encoded = bincode::serialize(&input).expect("serialize binary context");
        let decoded: OpcodeLabInput =
            bincode::deserialize(&encoded).expect("deserialize binary context");
        assert_eq!(decoded, input);
        for alternate in [
            OpcodeLabInput {
                tx_value: [0; 32],
                ..input.clone()
            },
            OpcodeLabInput {
                calldata: Vec::new(),
                ..input.clone()
            },
            OpcodeLabInput {
                block_timestamp: Some(18),
                ..input.clone()
            },
        ] {
            assert_ne!(bincode::serialize(&alternate).unwrap(), encoded);
        }
    }

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
