use serde::{Deserialize, Serialize};

#[derive(
    PartialEq, Eq, PartialOrd, Ord, Clone, Debug, Default, Deserialize, Serialize, Hash, Copy,
)]
#[repr(u8)]
pub enum ProofType {
    #[default]
    /// # Native
    ///
    /// This builds the block the same way the node does and then runs the result.
    #[serde(alias = "NATIVE")]
    Native = 0u8,
    /// # Sp1
    ///
    /// Uses the SP1 prover to build the block.
    #[serde(alias = "SP1")]
    Sp1 = 1u8,
    /// # Sgx
    ///
    /// Builds the block on a SGX supported CPU to create a proof.
    #[serde(alias = "SGX")]
    Sgx = 2u8,
    /// # Risc0
    ///
    /// Uses the RISC0 prover to build the block.
    #[serde(alias = "RISC0")]
    Risc0 = 3u8,
    /// # Tdx
    ///
    /// Builds the block inside a TDX-protected VM and produces an attestation quote.
    #[serde(alias = "TDX")]
    Tdx = 4u8,
}

impl std::fmt::Display for ProofType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ProofType::Native => "native",
            ProofType::Sp1 => "sp1",
            ProofType::Sgx => "sgx",
            ProofType::Risc0 => "risc0",
            ProofType::Tdx => "tdx",
        })
    }
}

impl std::str::FromStr for ProofType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "native" => Ok(ProofType::Native),
            "sp1" => Ok(ProofType::Sp1),
            "risc0" => Ok(ProofType::Risc0),
            "tdx" => Ok(ProofType::Tdx),
            _ => Err(format!("Unknown proof type {s}")),
        }
    }
}

impl TryFrom<u8> for ProofType {
    type Error = String;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Native),
            1 => Ok(Self::Sp1),
            3 => Ok(Self::Risc0),
            4 => Ok(Self::Tdx),
            _ => Err(format!("Unknown proof type {value}")),
        }
    }
}

/// Module for serializing `ProofType` as lowercase strings
pub mod lowercase {
    use super::ProofType;
    use serde::{Deserialize, Deserializer, Serializer};

    /// # Errors
    ///
    /// Returns an error when the proof type cannot be serialized as a lowercase string.
    pub fn serialize<S>(proof_type: &ProofType, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&proof_type.to_string())
    }

    /// # Errors
    ///
    /// Returns an error when the proof type string is unknown.
    pub fn deserialize<'de, D>(deserializer: D) -> Result<ProofType, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::ProofType;

    #[test]
    fn rejects_sgx_variants_by_string() {
        assert!("sgx".parse::<ProofType>().is_err());
        assert!("sgxgeth".parse::<ProofType>().is_err());
    }

    #[test]
    fn rejects_sgx_variants_by_u8() {
        assert!(ProofType::try_from(2u8).is_err());
        assert!(ProofType::try_from(4u8).is_err());
    }
}
