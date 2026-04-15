use alloy_primitives::B256;
use raiko2_primitives::ProofType;
use std::collections::{HashMap, VecDeque, hash_map::Entry};
use std::time::{Duration, SystemTime};

use crate::config::{ZkAnyConfig, ZkAnyTargetConfig};

const SEED_CACHE_CAPACITY: usize = 8_192;

#[derive(Debug, Clone)]
struct SamplingTargetState {
    probability: f64,
    per_day: u64,
    last_draw_at: Option<SystemTime>,
}

#[derive(Debug, Clone)]
pub(crate) struct ZkAnySampler {
    entries: Vec<(ProofType, SamplingTargetState)>,
    cached_results: HashMap<B256, Option<ProofType>>,
    cached_order: VecDeque<B256>,
}

impl ZkAnySampler {
    #[must_use]
    pub(crate) fn from_config(config: &ZkAnyConfig) -> Self {
        let entries = config
            .sampling_entries()
            .into_iter()
            .map(|(proof_type, target)| (proof_type, SamplingTargetState::from_config(&target)))
            .collect();
        Self {
            entries,
            cached_results: HashMap::new(),
            cached_order: VecDeque::new(),
        }
    }

    #[must_use]
    pub(crate) fn draw(&mut self, seed: B256) -> Option<ProofType> {
        let now = SystemTime::now();
        self.draw_with_time(seed, now)
    }

    #[must_use]
    fn draw_with_time(&mut self, seed: B256, now: SystemTime) -> Option<ProofType> {
        if let Some(result) = self.cached_results.get(&seed).copied() {
            return result;
        }

        let draw_result = self
            .draw_candidate(seed)
            .filter(|proof_type| self.check_frequency(*proof_type, now));
        self.cache_result(seed, draw_result);
        draw_result
    }

    #[must_use]
    fn draw_candidate(&self, seed: B256) -> Option<ProofType> {
        let draw_seed =
            u32::from_le_bytes(seed.as_slice()[28..32].try_into().expect("slice length"));
        let draw_ratio = f64::from(draw_seed) / f64::from(u32::MAX);
        let mut cumulative_probability = 0.0f64;

        for (proof_type, target) in &self.entries {
            cumulative_probability += target.probability;
            if draw_ratio < cumulative_probability {
                return Some(*proof_type);
            }
        }

        None
    }

    fn check_frequency(&mut self, proof_type: ProofType, now: SystemTime) -> bool {
        let Some((_, target)) = self
            .entries
            .iter_mut()
            .find(|(candidate, _)| *candidate == proof_type)
        else {
            return false;
        };

        if target.per_day == 0 {
            return true;
        }

        let min_interval = Duration::from_secs((86_400 + (target.per_day / 2)) / target.per_day);

        let should_draw = target.last_draw_at.is_none_or(|last_draw_at| {
            now.duration_since(last_draw_at)
                .map(|elapsed| elapsed >= min_interval)
                .unwrap_or(false)
        });

        if should_draw {
            target.last_draw_at = Some(now);
        }

        should_draw
    }

    fn cache_result(&mut self, seed: B256, result: Option<ProofType>) {
        if let Entry::Occupied(mut entry) = self.cached_results.entry(seed) {
            entry.insert(result);
            self.cached_order.retain(|candidate| *candidate != seed);
            self.cached_order.push_back(seed);
            return;
        }

        if self.cached_order.len() >= SEED_CACHE_CAPACITY
            && let Some(evicted) = self.cached_order.pop_front()
        {
            self.cached_results.remove(&evicted);
        }

        self.cached_order.push_back(seed);
        self.cached_results.insert(seed, result);
    }
}

impl SamplingTargetState {
    const fn from_config(config: &ZkAnyTargetConfig) -> Self {
        Self {
            probability: config.probability,
            per_day: config.per_day,
            last_draw_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ZkAnyConfig, ZkAnyTargetConfig};

    fn seeded(last_byte: u8) -> B256 {
        let mut bytes = [0u8; 32];
        bytes[31] = last_byte;
        B256::from(bytes)
    }

    #[test]
    fn sampler_returns_none_when_disabled() {
        let mut sampler = ZkAnySampler::from_config(&ZkAnyConfig::default());
        assert_eq!(sampler.draw(seeded(1)), None);
    }

    #[test]
    fn sampler_draws_sp1_for_full_probability() {
        let mut sampler = ZkAnySampler::from_config(&ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 1.0,
                per_day: 0,
            }),
            risc0: None,
        });
        assert_eq!(sampler.draw(seeded(7)), Some(ProofType::Sp1));
    }

    #[test]
    fn sampler_preserves_cached_result_for_same_seed() {
        let mut sampler = ZkAnySampler::from_config(&ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 1.0,
                per_day: 1,
            }),
            risc0: None,
        });
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let seed = seeded(9);
        let first = sampler.draw_with_time(seed, now);
        let second = sampler.draw_with_time(seed, now + Duration::from_secs(1));

        assert_eq!(first, Some(ProofType::Sp1));
        assert_eq!(second, Some(ProofType::Sp1));
    }

    #[test]
    fn sampler_applies_per_day_gate_for_new_seeds() {
        let mut sampler = ZkAnySampler::from_config(&ZkAnyConfig {
            sp1: Some(ZkAnyTargetConfig {
                probability: 1.0,
                per_day: 1,
            }),
            risc0: None,
        });
        let first = sampler.draw_with_time(seeded(1), SystemTime::UNIX_EPOCH);
        let second =
            sampler.draw_with_time(seeded(2), SystemTime::UNIX_EPOCH + Duration::from_secs(1));

        assert_eq!(first, Some(ProofType::Sp1));
        assert_eq!(second, None);
    }
}
